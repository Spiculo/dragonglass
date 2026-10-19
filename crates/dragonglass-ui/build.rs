//! Embeds the Windows icon and version resource into the app binary.

fn main() {
    #[cfg(windows)]
    {
        use std::path::Path;

        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let packaging = manifest.join("../../packaging/windows");
        let icon = packaging.join("dragon-glass.ico");
        let version_rc = packaging.join("dragon-glass.rc");

        println!("cargo:rerun-if-changed={}", icon.display());
        println!("cargo:rerun-if-changed={}", version_rc.display());

        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(&icon.to_string_lossy());
        resource.set_resource_file(&version_rc.to_string_lossy());

        if let Err(error) = resource.compile() {
            println!("cargo:warning=could not embed Windows resources: {error}");
        }
    }
}
