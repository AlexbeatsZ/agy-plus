fn main() {
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/app-icon.ico")
            .set("ProductName", "AGY Plus")
            .set(
                "FileDescription",
                "AGY Plus account switcher and Refresh Token sign-in launcher for Google Antigravity",
            )
            .set("CompanyName", "AlexbeatsZ")
            .set("OriginalFilename", "agy-plus.exe");
        resource
            .compile()
            .expect("failed to compile Windows resources");
    }
}
