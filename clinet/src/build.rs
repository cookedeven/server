fn main() {
    csbindgen::Builder::default()
        .input_extern_file("lib.rs")
        .csharp_dll_name("example")
        .generate_csharp_file("../dotnet/NativeMethods.g.cs")
        .unwrap();
}