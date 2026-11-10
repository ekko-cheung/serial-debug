fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Serial Debug");
        res.set("FileDescription", "Serial Debug Assistant");
        res.compile().expect("failed to embed icon");
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
