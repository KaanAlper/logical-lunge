//! Records what the screen actually shows (Desktop Duplication): for every
//! frame DWM presents on the primary monitor, its QPC present time and the x of
//! the bench's magenta marker on one pixel row.
//!   frame-capture <row y> <seconds> <out.csv>
use std::io::Write;
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

fn main() -> windows::core::Result<()> {
  let args: Vec<String> = std::env::args().collect();
  let row: u32 = args[1].parse().unwrap();
  let secs: f64 = args[2].parse().unwrap();
  let out = &args[3];
  unsafe {
    let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
    // the adapter and output holding the primary monitor (desktop origin 0,0)
    let mut found = None;
    let mut a = 0;
    while let Ok(adapter) = factory.EnumAdapters1(a) {
      let mut o = 0;
      while let Ok(output) = adapter.EnumOutputs(o) {
        let d = output.GetDesc()?;
        if d.DesktopCoordinates.left == 0 && d.DesktopCoordinates.top == 0 {
          found = Some((adapter.clone(), output));
        }
        o += 1;
      }
      a += 1;
    }
    let (adapter, output) = found.expect("no primary output");
    let mut device = None;
    let mut context = None;
    D3D11CreateDevice(&adapter, D3D_DRIVER_TYPE_UNKNOWN, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context))?;
    let device = device.unwrap();
    let context = context.unwrap();
    let output1: IDXGIOutput1 = output.cast()?;
    let dup = output1.DuplicateOutput(&device)?;
    let desc = dup.GetDesc();
    let width = desc.ModeDesc.Width;
    let staging_desc = D3D11_TEXTURE2D_DESC {
      Width: width, Height: 1, MipLevels: 1, ArraySize: 1, Format: DXGI_FORMAT_B8G8R8A8_UNORM,
      SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 }, Usage: D3D11_USAGE_STAGING, BindFlags: 0,
      CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, MiscFlags: 0,
    };
    let mut staging = None;
    device.CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
    let staging = staging.unwrap();
    let mut freq = 0i64;
    QueryPerformanceFrequency(&mut freq)?;
    let mut start = 0i64;
    QueryPerformanceCounter(&mut start)?;
    let end = start + (secs * freq as f64) as i64;
    let mut f = std::io::BufWriter::new(std::fs::File::create(out).unwrap());
    writeln!(f, "# qpc frequency {freq}").unwrap();
    writeln!(f, "qpc,accumulated,x").unwrap();
    let mut frames = 0;
    loop {
      let mut now = 0i64;
      QueryPerformanceCounter(&mut now)?;
      if now > end { break; }
      let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
      let mut res = None;
      match dup.AcquireNextFrame(100, &mut info, &mut res) {
        Ok(()) => {}
        Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
        Err(e) => { eprintln!("acquire: {e:?}"); break; }
      }
      if info.LastPresentTime != 0 {
        let tex: ID3D11Texture2D = res.unwrap().cast()?;
        let b = D3D11_BOX { left: 0, top: row, front: 0, right: width, bottom: row + 1, back: 1 };
        context.CopySubresourceRegion(&staging, 0, 0, 0, 0, &tex, 0, Some(&b));
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
        let px = std::slice::from_raw_parts(m.pData as *const u8, (width * 4) as usize);
        let mut x: i64 = -1;
        for i in 0..width as usize {
          let (b_, g, r) = (px[i * 4], px[i * 4 + 1], px[i * 4 + 2]);
          if r > 240 && b_ > 240 && g < 20 { x = i as i64; break; }
        }
        context.Unmap(&staging, 0);
        writeln!(f, "{},{},{}", info.LastPresentTime, info.AccumulatedFrames, x).unwrap();
        frames += 1;
      }
      let _ = dup.ReleaseFrame();
    }
    eprintln!("frames {frames}, qpc freq {freq}");
  }
  Ok(())
}
