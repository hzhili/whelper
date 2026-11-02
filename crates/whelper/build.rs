fn main() {
    println!("cargo:rerun-if-changed=../whelper-shell/assets/whelper.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../whelper-shell/assets/whelper.ico");
    resource
        .compile()
        .expect("failed to embed the whelper application icon");
}
