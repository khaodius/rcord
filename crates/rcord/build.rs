//! Embeds the icon and version information in the Windows executable.

fn main() {
    println!("cargo::rerun-if-changed=../../assets/icons/rcord.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icons/rcord.ico");
        res.set("ProductName", "rcord");
        res.set("FileDescription", "rcord");
        res.set("CompanyName", "khaodius");
        res.set("LegalCopyright", "Licensed under the Apache License 2.0");
        if let Err(e) = res.compile() {
            println!("cargo::warning=could not embed the Windows icon: {e}");
        }
    }
}
