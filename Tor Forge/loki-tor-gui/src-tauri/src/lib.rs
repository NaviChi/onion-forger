use std::sync::atomic::{AtomicBool, Ordering};

static IS_PROXY_RUNNING: AtomicBool = AtomicBool::new(false);

#[tauri::command]
async fn start_proxy() -> Result<u16, String> {
    if IS_PROXY_RUNNING.load(Ordering::SeqCst) {
        return Err("Proxy is already running".into());
    }

    // Attempt to find an open port starting at 9050
    let mut target_port = 9050;
    for port in 9050..9060 {
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            target_port = port;
            break;
        }
    }

    println!("Attempting to bootstrap Tor on port {}", target_port);

    match loki_tor_core::bootstrap_tor_daemon(target_port).await {
        Ok(_) => {
            IS_PROXY_RUNNING.store(true, Ordering::SeqCst);
            Ok(target_port)
        }
        Err(e) => Err(format!("Critical Failure: {}", e)),
    }
}

#[tauri::command]
fn get_proxy_status() -> String {
    if IS_PROXY_RUNNING.load(Ordering::SeqCst) {
        "Active".to_string()
    } else {
        "Disconnected".to_string()
    }
}

#[tauri::command]
fn panic_shutdown() {
    println!("PANIC SHUTDOWN invoked! Terminating process and severing connections immediately...");
    std::process::exit(1); // Non-zero exit to signify abrupt shutdown vs normal
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // Setup System Tray (Cross-Platform: Windows, Linux, macOS)
            #[cfg(desktop)]
            {
                use tauri::menu::{Menu, MenuItem};
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
                use tauri::Manager;

                let quit_i = MenuItem::with_id(app, "quit", "Quit LOKI Daemon", true, None::<&str>)?;
                let show_i = MenuItem::with_id(app, "show", "Show Dashboard", true, None::<&str>)?;
                
                let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

                let _tray = TrayIconBuilder::new()
                    .tooltip("LOKI Tor Core MANET Proxy")
                    .icon(app.default_window_icon().unwrap().clone())
                    .menu(&menu)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "quit" => {
                            println!("System Tray Quit requested. Terminating Daemon...");
                            app.exit(0);
                        }
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| match event {
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } => {
                            let app = tray.app_handle();
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        _ => {}
                    })
                    .build(app)?;
            }
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![start_proxy, get_proxy_status, panic_shutdown])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
