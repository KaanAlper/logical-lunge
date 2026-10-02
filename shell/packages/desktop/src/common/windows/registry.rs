use std::ffi::c_void;

use windows::{
  core::HSTRING,
  Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS},
    System::Registry::{
      RegGetValueW, HKEY, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE, RRF_RT_ANY,
      RRF_RT_REG_DWORD,
    },
  },
};

/// Logical Lunge: a string value of the registry (REG_SZ, or REG_EXPAND_SZ
/// with its variables expanded). `Ok(None)`: the key or the value does not
/// exist, or it is not a string. `Err`: the Win32 error code.
pub fn read_reg_string(
  root: HKEY,
  path: &str,
  name: &str,
) -> Result<Option<String>, u32> {
  let (path, name) = (HSTRING::from(path), HSTRING::from(name));
  let mut kind = REG_VALUE_TYPE::default();
  let mut bytes = 0u32;
  let status = unsafe {
    RegGetValueW(root, &path, &name, RRF_RT_ANY, Some(&mut kind), None, Some(&mut bytes))
  };
  if status == ERROR_FILE_NOT_FOUND {
    return Ok(None);
  }
  if status != ERROR_SUCCESS {
    return Err(status.0);
  }
  if kind != REG_SZ && kind != REG_EXPAND_SZ {
    return Ok(None);
  }
  // An expanded REG_EXPAND_SZ can be longer than the size asked first.
  for _ in 0..3 {
    let mut data = vec![0u16; (bytes as usize).div_ceil(2) + 1];
    bytes = (data.len() * 2) as u32;
    let status = unsafe {
      RegGetValueW(
        root,
        &path,
        &name,
        RRF_RT_ANY,
        Some(&mut kind),
        Some(data.as_mut_ptr().cast::<c_void>()),
        Some(&mut bytes),
      )
    };
    if status == ERROR_MORE_DATA {
      continue;
    }
    if status != ERROR_SUCCESS {
      return Err(status.0);
    }
    let end = data.iter().position(|&c| c == 0).unwrap_or(data.len());
    return Ok(Some(String::from_utf16_lossy(&data[..end])));
  }
  Err(ERROR_MORE_DATA.0)
}

/// Logical Lunge: a DWORD value of the registry; `None` when the key or the
/// value does not exist or is not a DWORD.
pub fn read_reg_dword(root: HKEY, path: &str, name: &str) -> Option<u32> {
  let (path, name) = (HSTRING::from(path), HSTRING::from(name));
  let mut value = 0u32;
  let mut bytes = std::mem::size_of::<u32>() as u32;
  let status = unsafe {
    RegGetValueW(
      root,
      &path,
      &name,
      RRF_RT_REG_DWORD,
      None,
      Some((&mut value as *mut u32).cast::<c_void>()),
      Some(&mut bytes),
    )
  };
  (status == ERROR_SUCCESS).then_some(value)
}
