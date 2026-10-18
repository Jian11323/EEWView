fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../icon/brand/app.ico");
        res.set("ProductName", "StarQuake");
        res.set("FileDescription", "StarQuake");
        res.set("LegalCopyright", "Apache-2.0");
        if let Err(e) = res.compile() {
            println!("cargo:warning=winres failed: {e}");
        }
    }
    println!("cargo:rerun-if-changed=../../icon/brand/app.ico");
}
