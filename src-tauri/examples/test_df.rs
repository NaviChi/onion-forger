use scraper::{Html, Selector};

fn main() {
    let html = r#"
<html>
<body>
    <div class="companies">
        <div class="item">
            <div class="name"><a class="text-pointer-animations dir" href="/?path=RJZ0104&token=eyJhbGciOiJSUzUxMiIsInR5cCI6IkpXVCJ9.eyJjb2xvcl9">RJZ0104/</a></div>
        </div>
        <div class="item">
            <div class="name"><a class="text-pointer-animations" href="/download?path=file.txt&token=foo">file.txt (123 bytes)</a></div>
            <div class="size">123 bytes</div>
        </div>
    </div>
</body>
</html>
    "#;

    let document = Html::parse_document(html);
    let item_selector = Selector::parse(".item").unwrap();
    let link_selector = Selector::parse("a.text-pointer-animations").unwrap();
    let _size_selector = Selector::parse("div.size").unwrap();

    let mut count = 0;
    for item in document.select(&item_selector) {
        if let Some(link) = item.select(&link_selector).next() {
            let is_dir = link.value().classes().any(|c| c == "dir");
            let href = link.value().attr("href").unwrap_or("");
            count += 1;
            println!("Found item! dir={}, href={}", is_dir, href);
        } else {
            println!("Item has no matching link");
        }
    }
    println!("Total matched: {}", count);
}
