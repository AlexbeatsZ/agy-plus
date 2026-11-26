#[cfg(target_os = "windows")]
fn validate_windows_credential_size(size: usize) -> Result<(), String> {
    const CRED_MAX_CREDENTIAL_BLOB_SIZE: usize = 5 * 512;
    if size > CRED_MAX_CREDENTIAL_BLOB_SIZE {
        return Err("ANTIGRAVITY_WINDOWS_CREDENTIAL_TOO_LARGE".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_credential_write_error(error: std::io::Error) -> String {
    const ERROR_NOT_ENOUGH_MEMORY: i32 = 8;
    if error.raw_os_error() == Some(ERROR_NOT_ENOUGH_MEMORY) {
        return "ANTIGRAVITY_WINDOWS_CREDENTIAL_STORE_CAPACITY".to_string();
    }
    format!("写入 Windows Credential Manager 失败: {error}")
}

pub fn write_antigravity_system_payload(payload_json: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    write_antigravity_keychain_payload(
        payload_json,
        std::ffi::OsStr::new("security"),
        std::time::Duration::from_secs(15),
    )?;

    #[cfg(target_os = "windows")]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use std::ptr;

        #[repr(C)]
        struct FileTime {
            dw_low_date_time: u32,
            dw_high_date_time: u32,
        }

        #[repr(C)]
        struct CredentialW {
            flags: u32,
            cred_type: u32,
            target_name: *const u16,
            comment: *const u16,
            last_written: FileTime,
            credential_blob_size: u32,
            credential_blob: *const u8,
            persist: u32,
            attribute_count: u32,
            attributes: *const std::ffi::c_void,
            target_alias: *const u16,
            user_name: *const u16,
        }

        #[link(name = "advapi32")]
        extern "system" {
            fn CredWriteW(credential: *const CredentialW, flags: u32) -> i32;
        }

        const CRED_TYPE_GENERIC: u32 = 1;
        const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;

        let target_wide: Vec<u16> = OsStr::new("gemini:antigravity")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let user_wide: Vec<u16> = OsStr::new("antigravity")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let secret = payload_json.as_bytes();
        validate_windows_credential_size(secret.len())?;

        let credential = CredentialW {
            flags: 0,
            cred_type: CRED_TYPE_GENERIC,
            target_name: target_wide.as_ptr(),
            comment: ptr::null(),
            last_written: FileTime {
                dw_low_date_time: 0,
                dw_high_date_time: 0,
            },
            credential_blob_size: secret.len() as u32,
            credential_blob: secret.as_ptr(),
            persist: CRED_PERSIST_LOCAL_MACHINE,
            attribute_count: 0,
            attributes: ptr::null(),
            target_alias: ptr::null(),
            user_name: user_wide.as_ptr(),
        };

        unsafe {
            // CredWriteW replaces an existing target without deleting it first.
            if CredWriteW(&credential, 0) == 0 {
                return Err(windows_credential_write_error(
                    std::io::Error::last_os_error(),
                ));
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;

        let mut command = Command::new("secret-tool");
        command.args([
            "store",
            "--label=gemini",
            "service",
            "gemini",
            "username",
            "antigravity",
        ]);
        let output = crate::process_timeout::output_with_input_timeout(
            &mut command,
            payload_json.as_bytes(),
            std::time::Duration::from_secs(15),
        )
        .map_err(|e| format!("执行 Linux secret-tool 写入命令失败: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Linux secret-tool 写入失败: {}", stderr.trim()));
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn write_antigravity_keychain_payload(
    payload_json: &str,
    program: &std::ffi::OsStr,
    timeout: std::time::Duration,
) -> Result<(), String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use std::process::Command;

    let keychain_value = format!("go-keyring-base64:{}", STANDARD.encode(payload_json));
    // Update the credential in place and preserve its existing access policy.
    // Passing -A (or -T) also calls SecKeychainItemSetAccess, which prompts for
    // authorization on every switch even when the password update is allowed.
    let mut command = Command::new(program);
    command.args([
        "add-generic-password",
        "-U",
        "-s",
        "gemini",
        "-a",
        "antigravity",
        "-w",
        &keychain_value,
    ]);
    let output = crate::process_timeout::output_with_timeout(&mut command, timeout)
        .map_err(|e| format!("执行 macOS Keychain 写入命令失败: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("写入 macOS Keychain 失败: {}", stderr.trim()));
    }
    Ok(())
}
