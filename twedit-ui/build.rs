fn main() {
    windows_reactor_setup::as_self_contained();
    embed_resource::compile("app.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("failed to embed app.rc");
}
