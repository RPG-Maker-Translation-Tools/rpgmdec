fn main() {
    embed_resource::compile("icons/icon.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
