fn main() {
    // Only embed icon on Windows
    #[cfg(target_os = "windows")]
    {
        use std::fs::File;
        use std::io::BufWriter;
        use std::path::Path;

        let out_dir = std::env::var("OUT_DIR").unwrap();
        let ico_path = Path::new(&out_dir).join("app.ico");

        // Create ICO from PNG files
        let png_files = [
            "icons/app-32.png",
            "icons/app-180.png",
            "icons/app-192.png",
        ];

        let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);

        for png_path in &png_files {
            if Path::new(png_path).exists() {
                let file = File::open(png_path).expect("Failed to open PNG");
                let image = ico::IconImage::read_png(file).expect("Failed to read PNG");
                icon_dir.add_entry(ico::IconDirEntry::encode(&image).expect("Failed to encode icon"));
            }
        }

        let ico_file = File::create(&ico_path).expect("Failed to create ICO file");
        icon_dir.write(BufWriter::new(ico_file)).expect("Failed to write ICO");

        // Embed the icon using winresource
        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico_path.to_str().unwrap());
        res.compile().expect("Failed to compile Windows resources");
    }
}
