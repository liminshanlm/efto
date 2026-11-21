fn main() {
    // tauri-build 会读取 tauri.conf.json、生成窗口的 Windows 资源（需要 icons/icon.ico 与 rc.exe）
    tauri_build::build()
}
