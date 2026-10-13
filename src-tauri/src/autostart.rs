//! 开机自启：读写当前用户的 Run 注册表项，不需要额外插件或管理员权限。

#[cfg(target_os = "windows")]
mod imp {
    use anyhow::{Context, Result};
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "Zhizhi";

    fn run_key(flags: u32) -> Result<RegKey> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_KEY, flags)
            .context("打不开开机启动项注册表")
    }

    pub fn enabled() -> Result<bool> {
        let key = run_key(KEY_READ)?;
        let value: Result<String, _> = key.get_value(VALUE_NAME);
        Ok(value.is_ok())
    }

    pub fn set(on: bool) -> Result<()> {
        if !on {
            if enabled()? {
                run_key(KEY_WRITE)?.delete_value(VALUE_NAME).context("删除开机启动项失败")?;
            }
            return Ok(());
        }
        let exe = std::env::current_exe().context("找不到程序自身路径")?;
        // 路径可能含空格，注册表里必须带引号
        let command = format!("\"{}\"", exe.display());
        RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(RUN_KEY)
            .context("打不开开机启动项注册表")?
            .0
            .set_value(VALUE_NAME, &command)
            .context("写入开机启动项失败")?;
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use anyhow::{anyhow, Result};

    pub fn enabled() -> Result<bool> {
        Ok(false)
    }

    pub fn set(_on: bool) -> Result<()> {
        Err(anyhow!("开机自启只在 Windows 上支持"))
    }
}

pub use imp::{enabled, set};
