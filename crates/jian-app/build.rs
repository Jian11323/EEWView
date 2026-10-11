fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../assets/brand/app.ico");
        res.set("ProductName", "EEW View");
        res.set("FileDescription", "EEW View");
        res.set("LegalCopyright", "Apache-2.0");
        if let Err(e) = res.compile() {
            println!("cargo:warning=winres failed: {e}");
        }
    }
    println!("cargo:rerun-if-changed=../../assets/brand/app.ico");
}
