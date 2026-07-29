fn main() {
    let document = kemuri_server::openapi_document();
    println!(
        "{}",
        serde_json::to_string_pretty(&document).expect("OpenAPI document must serialize")
    );
}
