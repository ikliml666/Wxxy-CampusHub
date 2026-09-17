//! DPAPI 账号密码加密（Windows CurrentUser 作用域，跨用户不可解密）。
//!
//! 裸 FFI 实现（`extern "system"` + `#[link(name = "crypt32")]`）拷贝自参考项目
//! Wxxy-CampusLogin src-tauri/src/account/crypto.rs，零新依赖（不引入 windows crate）。

#[cfg(target_os = "windows")]
mod dpapi {
    use std::ptr;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            data_in: *mut DataBlob,
            data_descr: *const u16,
            optional_entropy: *mut DataBlob,
            reserved: *mut std::ffi::c_void,
            prompt_struct: *mut std::ffi::c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;

        fn CryptUnprotectData(
            data_in: *mut DataBlob,
            data_descr: *mut *mut u16,
            optional_entropy: *mut DataBlob,
            reserved: *mut std::ffi::c_void,
            prompt_struct: *mut std::ffi::c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(h_mem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    /// 调用 DPAPI 函数（CryptProtectData/CryptUnprotectData）的通用 helper，
    /// 统一 DataBlob 构造、结果检查、输出数据读取与 LocalFree 释放。
    fn call_dpapi<F>(input_data: &[u8], dpapi_call: F, error_msg: &str, empty_error_msg: &str) -> Result<Vec<u8>, String>
    where
        F: FnOnce(&mut DataBlob, &mut DataBlob) -> i32,
    {
        let mut input_owned = input_data.to_vec();
        let mut input = DataBlob {
            cb_data: input_owned.len() as u32,
            pb_data: input_owned.as_mut_ptr(),
        };
        let mut output = DataBlob {
            cb_data: 0,
            pb_data: ptr::null_mut(),
        };

        let result = dpapi_call(&mut input, &mut output);

        if result == 0 {
            // 失败路径也要释放输出缓冲：多数实现失败时 pb_data 为 null，
            // 但契约上不保证，判空后与成功路径同款清理
            if !output.pb_data.is_null() {
                unsafe { LocalFree(output.pb_data as *mut std::ffi::c_void) };
            }
            return Err(error_msg.to_string());
        }

        if output.pb_data.is_null() || output.cb_data == 0 {
            unsafe { LocalFree(output.pb_data as *mut std::ffi::c_void) };
            return Err(empty_error_msg.to_string());
        }
        let data = unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize).to_vec() };
        unsafe { LocalFree(output.pb_data as *mut std::ffi::c_void) };
        Ok(data)
    }

    pub fn encrypt(plaintext: &[u8]) -> Result<Vec<u8>, String> {
        call_dpapi(
            plaintext,
            |input, output| unsafe {
                CryptProtectData(
                    input,
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    0,
                    output,
                )
            },
            "DPAPI加密失败",
            "DPAPI加密返回空数据",
        )
    }

    pub fn decrypt(data: &[u8]) -> Result<Vec<u8>, String> {
        call_dpapi(
            data,
            |input, output| unsafe {
                CryptUnprotectData(
                    input,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    0,
                    output,
                )
            },
            "DPAPI解密失败，可能需要重新输入密码",
            "DPAPI解密返回空数据",
        )
    }
}

/// 明文 → DPAPI 密文 base64（落盘 password_b64 / session cookie 值形态）。
#[cfg(target_os = "windows")]
pub fn dpapi_protect(plaintext: &str) -> Result<String, String> {
    let encrypted = dpapi::encrypt(plaintext.as_bytes())?;
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &encrypted,
    ))
}

/// DPAPI 密文 base64 → 明文。
#[cfg(target_os = "windows")]
pub fn dpapi_unprotect(encrypted_base64: &str) -> Result<String, String> {
    let data = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        encrypted_base64,
    )
    .map_err(|e| format!("Base64解码失败: {e}"))?;
    let decrypted = dpapi::decrypt(&data)?;
    String::from_utf8(decrypted).map_err(|e| format!("UTF8转换失败: {e}"))
}

// 非桌面平台无 DPAPI:阶段 1 安卓密码不落盘,加密路径不触达;阶段 2 换 Android Keystore
#[cfg(not(target_os = "windows"))]
pub fn dpapi_protect(_plaintext: &str) -> Result<String, String> {
    Err("加密存储仅桌面端支持".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn dpapi_unprotect(_encrypted_base64: &str) -> Result<String, String> {
    Err("加密存储仅桌面端支持".to_string())
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn dpapi_roundtrip() {
        let plain = "Test@Password123 密码含中文";
        let cipher = dpapi_protect(plain).unwrap();
        assert!(!cipher.is_empty());
        // 密文不含明文子串
        assert!(!cipher.contains(plain));
        assert_eq!(dpapi_unprotect(&cipher).unwrap(), plain);
    }

    #[test]
    fn dpapi_rejects_invalid_base64() {
        assert!(dpapi_unprotect("!!!非法base64!!!").is_err());
    }
}
