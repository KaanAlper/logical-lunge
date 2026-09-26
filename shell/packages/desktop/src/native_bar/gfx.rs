//! Direct3D 11 + Direct2D + DirectComposition + DirectWrite + WIC objects,
//! shared by every bar window (one device per process: each device costs
//! driver memory).

use windows::{
  core::{Interface, Result},
  Foundation::Numerics::Matrix3x2,
  Win32::{
    Foundation::{HMODULE, POINT},
    Graphics::{
      Direct2D::{
        Common::{D2D1_COLOR_F, D2D_POINT_2F, D2D_RECT_F},
        D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext,
        ID2D1Factory1, ID2D1SolidColorBrush, D2D1_BITMAP_PROPERTIES1,
        D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_ELLIPSE,
        D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_ROUNDED_RECT,
      },
      Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP},
      Direct3D11::{
        D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_SDK_VERSION,
      },
      DirectComposition::{
        DCompositionCreateDevice2, IDCompositionDesktopDevice,
        IDCompositionSurface,
      },
      DirectWrite::{DWriteCreateFactory, IDWriteFactory6, DWRITE_FACTORY_TYPE_SHARED},
      Dxgi::{
        Common::{DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM},
        IDXGIDevice,
      },
      Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA,
        IWICImagingFactory, WICBitmapDitherTypeNone,
        WICBitmapPaletteTypeMedianCut, WICDecodeMetadataCacheOnDemand,
      },
    },
    System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER},
  },
};

pub struct Gfx {
  pub d2d: ID2D1Device,
  /// Device-level resources (brushes, bitmaps) are created here and are
  /// usable in every surface's `BeginDraw` context of the same device.
  pub dc: ID2D1DeviceContext,
  pub dcomp: IDCompositionDesktopDevice,
  pub dwrite: IDWriteFactory6,
  pub wic: IWICImagingFactory,
  /// No usable GPU: Direct2D renders in software (still faster than GDI+).
  pub warp: bool,
}

impl Gfx {
  pub fn new() -> Result<Self> {
    unsafe {
      let mut d3d: Option<ID3D11Device> = None;
      let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
      let mut warp = false;
      let hw = D3D11CreateDevice(
        None,
        D3D_DRIVER_TYPE_HARDWARE,
        HMODULE::default(),
        flags,
        None,
        D3D11_SDK_VERSION,
        Some(&mut d3d),
        None,
        None,
      );
      if hw.is_err() || d3d.is_none() {
        warp = true;
        D3D11CreateDevice(
          None,
          D3D_DRIVER_TYPE_WARP,
          HMODULE::default(),
          flags,
          None,
          D3D11_SDK_VERSION,
          Some(&mut d3d),
          None,
          None,
        )?;
      }
      let d3d = d3d.ok_or_else(|| windows::core::Error::from_win32())?;
      let dxgi: IDXGIDevice = d3d.cast()?;
      let factory: ID2D1Factory1 =
        D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
      let d2d = factory.CreateDevice(&dxgi)?;
      let dc = d2d.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
      let dcomp: IDCompositionDesktopDevice = DCompositionCreateDevice2(&d2d)?;
      let dwrite: IDWriteFactory6 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
      let wic: IWICImagingFactory =
        CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;

      Ok(Self { d2d, dc, dcomp, dwrite, wic, warp })
    }
  }

  /// A premultiplied BGRA surface of `w` x `h` physical pixels.
  pub fn surface(&self, w: u32, h: u32) -> Result<IDCompositionSurface> {
    unsafe {
      self.dcomp.CreateSurface(
        w.max(1),
        h.max(1),
        DXGI_FORMAT_B8G8R8A8_UNORM,
        DXGI_ALPHA_MODE_PREMULTIPLIED,
      )
    }
  }

  pub fn brush(&self, c: Rgba) -> Result<ID2D1SolidColorBrush> {
    unsafe { self.dc.CreateSolidColorBrush(&c.into(), None) }
  }

  /// Decodes PNG / ICO / JPEG bytes into a bitmap usable in any surface.
  pub fn bitmap(&self, bytes: &[u8]) -> Result<ID2D1Bitmap1> {
    unsafe {
      let stream = self.wic.CreateStream()?;
      stream.InitializeFromMemory(bytes)?;
      let decoder = self.wic.CreateDecoderFromStream(
        &stream,
        std::ptr::null(),
        WICDecodeMetadataCacheOnDemand,
      )?;
      let frame = decoder.GetFrame(0)?;
      let conv = self.wic.CreateFormatConverter()?;
      conv.Initialize(
        &frame,
        &GUID_WICPixelFormat32bppPBGRA,
        WICBitmapDitherTypeNone,
        None,
        0.0,
        WICBitmapPaletteTypeMedianCut,
      )?;
      self.dc.CreateBitmapFromWicBitmap(&conv, None::<*const D2D1_BITMAP_PROPERTIES1>)
    }
  }
}

/// Draws into `surface` with coordinates in DIPs (`scale` = DPI / 96).
pub fn draw_surface<F>(surface: &IDCompositionSurface, scale: f32, f: F) -> Result<()>
where
  F: FnOnce(&ID2D1DeviceContext) -> Result<()>,
{
  unsafe {
    let mut off = POINT::default();
    let dc: ID2D1DeviceContext = surface.BeginDraw(None, &mut off)?;
    dc.SetDpi(96.0 * scale, 96.0 * scale);
    dc.SetTransform(&Matrix3x2::translation(
      off.x as f32 / scale,
      off.y as f32 / scale,
    ));
    dc.Clear(Some(&Rgba(0, 0, 0, 0.0).into()));
    let res = f(&dc);
    let end = surface.EndDraw();
    res.and(end)
  }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub f32);

impl Rgba {
  pub const fn hex(v: u32) -> Self {
    Rgba((v >> 16) as u8, (v >> 8) as u8, v as u8, 1.0)
  }
  pub const fn alpha(self, a: f32) -> Self {
    Rgba(self.0, self.1, self.2, a)
  }
}

impl From<Rgba> for D2D1_COLOR_F {
  fn from(c: Rgba) -> Self {
    D2D1_COLOR_F {
      r: c.0 as f32 / 255.0,
      g: c.1 as f32 / 255.0,
      b: c.2 as f32 / 255.0,
      a: c.3,
    }
  }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
  pub x: f32,
  pub y: f32,
  pub w: f32,
  pub h: f32,
}

impl Rect {
  pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
    Self { x, y, w, h }
  }
  pub fn right(&self) -> f32 {
    self.x + self.w
  }
  pub fn bottom(&self) -> f32 {
    self.y + self.h
  }
  pub fn contains(&self, px: f32, py: f32) -> bool {
    px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
  }
  pub fn inset(&self, dx: f32, dy: f32) -> Self {
    Self::new(self.x + dx, self.y + dy, self.w - 2.0 * dx, self.h - 2.0 * dy)
  }
  pub fn d2d(&self) -> D2D_RECT_F {
    D2D_RECT_F { left: self.x, top: self.y, right: self.right(), bottom: self.bottom() }
  }
  pub fn rounded(&self, r: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT { rect: self.d2d(), radiusX: r, radiusY: r }
  }
}

pub fn pt(x: f32, y: f32) -> D2D_POINT_2F {
  D2D_POINT_2F { x, y }
}

pub fn ellipse(cx: f32, cy: f32, r: f32) -> D2D1_ELLIPSE {
  D2D1_ELLIPSE { point: pt(cx, cy), radiusX: r, radiusY: r }
}
