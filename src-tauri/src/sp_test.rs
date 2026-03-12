#[test]
fn test_spillover_push_pop() {
    let q = crate::spillover::SpilloverQueue::<String>::new();
    q.push("https://example.com/site/data?uuid=foo-bar".to_string());
    let item = q.pop();
    assert_eq!(item.unwrap(), "https://example.com/site/data?uuid=foo-bar");
}
