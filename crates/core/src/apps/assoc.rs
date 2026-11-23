//! 把插件的扩展名注册到 Windows「打开方式」。
//!
//! 只写 `HKCU\Software\Classes`：免管理员、不改默认程序，用户仍可在
//! 资源管理器里选别的程序打开；卸载/取消注册时只删自己写的项。
//!
//! 注册后 `efto.exe --open-file "<路径>"` 由宿主按扩展名路由到插件，
//! 协议见 `docs/plugin-development.md`。

use crate::error::{CoreError, Result};

/// 注册表里的 ProgID。带 `efto.` 前缀，避免和其他程序撞名。
pub fn prog_id(plugin_id: &str) -> String {
    format!("efto.{plugin_id}")
}

fn check_plugin_id(plugin_id: &str) -> Result<()> {
    if plugin_id.trim().is_empty() || plugin_id.contains('\\') || plugin_id.contains('/') {
        return Err(CoreError::Assoc(format!("插件 id 不能用于注册表项: {plugin_id}")));
    }
    Ok(())
}

/// 打开文件时交给系统的命令行。
fn command_line(exe: &str, plugin_id: &str) -> String {
    format!("\"{exe}\" --open-file \"%1\" --plugin \"{plugin_id}\"")
}

/// 注册扩展名（HKCU）。`exts` 是已规范化的小写扩展名（不含点）。
#[cfg(windows)]
pub fn register(plugin_id: &str, title: &str, exe: &str, exts: &[String]) -> Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
    use winreg::RegKey;

    check_plugin_id(plugin_id)?;
    let progid = prog_id(plugin_id);
    let classes = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Software\Classes", KEY_WRITE)
        .map_err(|e| CoreError::Assoc(format!("打开 HKCU\\Software\\Classes 失败: {e}")))?;

    let (key, _) = classes
        .create_subkey(format!(r"{progid}\DefaultIcon"))
        .map_err(|e| CoreError::Assoc(format!("创建 {progid} 失败: {e}")))?;
    key.set_value("", &format!("\"{exe}\",0"))
        .map_err(|e| CoreError::Assoc(format!("写 {progid} 图标失败: {e}")))?;
    drop(key);

    let (cmd, _) = classes
        .create_subkey(format!(r"{progid}\shell\open\command"))
        .map_err(|e| CoreError::Assoc(format!("创建 {progid} 命令失败: {e}")))?;
    cmd.set_value("", &command_line(exe, plugin_id))
        .map_err(|e| CoreError::Assoc(format!("写 {progid} 命令失败: {e}")))?;
    drop(cmd);

    // ProgID 的默认值就是「打开方式」里显示的名字
    let (root, _) = classes
        .create_subkey(&progid)
        .map_err(|e| CoreError::Assoc(format!("创建 {progid} 失败: {e}")))?;
    root.set_value("", &title.to_string())
        .map_err(|e| CoreError::Assoc(format!("写 {progid} 名称失败: {e}")))?;
    drop(root);

    for ext in exts {
        let (ow, _) = classes
            .create_subkey(format!(r".{ext}\OpenWithProgids"))
            .map_err(|e| CoreError::Assoc(format!("创建 .{ext} 打开方式失败: {e}")))?;
        // 习惯上用 REG_NONE 空值登记，资源管理器只认名字不认内容
        ow.set_raw_value(
            &progid,
            &winreg::RegValue {
                bytes: Vec::new(),
                vtype: winreg::enums::REG_NONE,
            },
        )
        .map_err(|e| CoreError::Assoc(format!("登记 .{ext} 打开方式失败: {e}")))?;
    }
    Ok(())
}

/// 取消注册：只删 efto 自己写的 ProgID 与 OpenWithProgids 项，不动 `.md` 键本身。
#[cfg(windows)]
pub fn unregister(plugin_id: &str, exts: &[String]) -> Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_WRITE};
    use winreg::RegKey;

    check_plugin_id(plugin_id)?;
    let progid = prog_id(plugin_id);
    let classes = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Software\Classes", KEY_WRITE)
        .map_err(|e| CoreError::Assoc(format!("打开 HKCU\\Software\\Classes 失败: {e}")))?;

    for ext in exts {
        if let Ok(ow) = classes.open_subkey_with_flags(format!(r".{ext}\OpenWithProgids"), KEY_WRITE) {
            let _ = ow.delete_value(&progid);
        }
    }
    if let Ok(key) = classes.open_subkey_with_flags(&progid, KEY_ALL_ACCESS) {
        delete_tree(&key);
    }
    let _ = classes.delete_subkey(&progid);
    Ok(())
}

/// 递归清空子项与值，保证父项可以被删除。
#[cfg(windows)]
fn delete_tree(key: &winreg::RegKey) {
    use winreg::enums::KEY_ALL_ACCESS;
    for name in key.enum_keys().flatten() {
        if let Ok(sub) = key.open_subkey_with_flags(&name, KEY_ALL_ACCESS) {
            delete_tree(&sub);
        }
        let _ = key.delete_subkey(&name);
    }
    for (name, _) in key.enum_values().flatten() {
        let _ = key.delete_value(&name);
    }
}

/// 是否已按当前 exe 注册（用于设置页显示状态）。
#[cfg(windows)]
pub fn is_registered(plugin_id: &str, exe: &str, exts: &[String]) -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    if check_plugin_id(plugin_id).is_err() || exts.is_empty() {
        return false;
    }
    let progid = prog_id(plugin_id);
    let classes = match RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Software\Classes", KEY_READ)
    {
        Ok(k) => k,
        Err(_) => return false,
    };
    let cmd = classes
        .open_subkey_with_flags(format!(r"{progid}\shell\open\command"), KEY_READ)
        .and_then(|k| k.get_value::<String, _>(""))
        .ok();
    if cmd.as_deref() != Some(command_line(exe, plugin_id).as_str()) {
        return false;
    }
    exts.iter().all(|ext| {
        classes
            .open_subkey_with_flags(format!(r".{ext}\OpenWithProgids"), KEY_READ)
            .map(|k| k.get_raw_value(&progid).is_ok())
            .unwrap_or(false)
    })
}

#[cfg(not(windows))]
pub fn register(_plugin_id: &str, _title: &str, _exe: &str, _exts: &[String]) -> Result<()> {
    Err(CoreError::Assoc("当前平台不支持注册「打开方式」".into()))
}

#[cfg(not(windows))]
pub fn unregister(_plugin_id: &str, _exts: &[String]) -> Result<()> {
    Err(CoreError::Assoc("当前平台不支持注册「打开方式」".into()))
}

#[cfg(not(windows))]
pub fn is_registered(_plugin_id: &str, _exe: &str, _exts: &[String]) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prog_id_is_prefixed() {
        assert_eq!(prog_id("markdown"), "efto.markdown");
    }

    #[test]
    fn rejects_ids_with_path_separators() {
        assert!(check_plugin_id("a/b").is_err());
        assert!(check_plugin_id("a\\b").is_err());
        assert!(check_plugin_id("  ").is_err());
        assert!(check_plugin_id("markdown").is_ok());
    }

    #[test]
    fn command_line_quotes_paths() {
        let line = command_line(r"C:\Program Files\efto\efto.exe", "markdown");
        assert!(line.starts_with(r#""C:\Program Files\efto\efto.exe" --open-file "%1""#));
        assert!(line.ends_with(r#"--plugin "markdown""#));
    }
}
