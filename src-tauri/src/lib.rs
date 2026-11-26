mod credential;
mod login;
mod process_timeout;

#[tauri::command]
async fn login_with_rt(refresh_token: String) -> Result<login::LoginResult, String> {
    login::login(refresh_token).await
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![login_with_rt])
        .run(tauri::generate_context!())
        .expect("启动 AGY Plus 失败");
}
