//! Embeds the Windows icon and version resource into the app binary.
//!
//! `winresource` can generate a VERSIONINFO block and an icon, but `set_resource_file`
//! makes it compile only the supplied `.rc` file. We therefore keep the hand-written
//! version block and append the icon line (with an absolute path) before compiling.

fn main() {
    #[cfg(windows)]
    {
        use std::path::{Path, PathBuf};

        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let packaging = manifest.join("../../packaging/windows");
        let icon = packaging.join("dragon-glass.ico");
        let version_rc = packaging.join("dragon-glass.rc");

        println!("cargo:rerun-if-changed={}", icon.display());
        println!("cargo:rerun-if-changed={}", version_rc.display());

        // rc.exe resolves relative paths against its working directory, which is not the
        // packaging folder. Use an absolute, forward-slash path inside the generated rc.
        let icon_path = icon.to_string_lossy().replace('\\', "/");
        let mut script = std::fs::read_to_string(&version_rc).unwrap_or_default();
        script.push_str(&format!("\n1 ICON \"{icon_path}\"\n"));

        let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set"));
        let generated = out_dir.join("dragon-glass.rc");
        if let Err(error) = std::fs::write(&generated, script) {
            println!("cargo:warning=could not write generated Windows resource: {error}");
            return;
        }

        let mut resource = winresource::WindowsResource::new();
        resource.set_resource_file(&generated.to_string_lossy());
        if let Err(error) = resource.compile() {
            println!("cargo:warning=could not embed Windows resources: {error}");
        }
    }
}
