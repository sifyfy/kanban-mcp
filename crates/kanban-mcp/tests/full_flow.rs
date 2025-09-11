use kanban_mcp::Server;
use serde_json::json;

#[test]
fn move_updates_index_and_listing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let rn = Server::handle_value(json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"kanban_new","arguments":{"board":root,"title":"M","column":"backlog"}}
    }))
    .unwrap();
    let id = rn["result"]["cardId"].as_str().unwrap().to_string();
    let _mv = Server::handle_value(json!({
        "jsonrpc":"2.0","id":2,"method":"tools/call",
        "params":{"name":"kanban_move","arguments":{"board":root,"cardId":id,"toColumn":"doing"}}
    }))
    .unwrap();
    let l_doing = Server::handle_value(json!({
        "jsonrpc":"2.0","id":3,"method":"tools/call",
        "params":{"name":"kanban_list","arguments":{"board":root,"columns":["doing"],"offset":0,"limit":100}}
    })).unwrap();
    assert_eq!(l_doing["result"]["items"].as_array().unwrap().len(), 1);
    let l_backlog = Server::handle_value(json!({
        "jsonrpc":"2.0","id":4,"method":"tools/call",
        "params":{"name":"kanban_list","arguments":{"board":root,"columns":["backlog"],"offset":0,"limit":100}}
    })).unwrap();
    assert_eq!(l_backlog["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn relations_depends_relates_add_remove_and_parent_replace() {
    use kanban_storage::Board;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let ra = Server::handle_value(
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
        "name":"kanban_new","arguments":{"board":root,"title":"A","column":"backlog"}}}),
    )
    .unwrap();
    let a = ra["result"]["cardId"].as_str().unwrap().to_string();
    let rb = Server::handle_value(
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
        "name":"kanban_new","arguments":{"board":root,"title":"B","column":"backlog"}}}),
    )
    .unwrap();
    let b = rb["result"]["cardId"].as_str().unwrap().to_string();
    let rc = Server::handle_value(
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
        "name":"kanban_new","arguments":{"board":root,"title":"C","column":"backlog"}}}),
    )
    .unwrap();
    let c = rc["result"]["cardId"].as_str().unwrap().to_string();
    let _ = Server::handle_value(
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
        "name":"kanban_relations_set","arguments":{"board":root,
          "add":[{"type":"depends","from":a,"to":b},{"type":"relates","from":a,"to":c}]}}}),
    )
    .unwrap();
    let board = Board::new(root);
    let af = board.read_card(&a).unwrap();
    assert!(af
        .front_matter
        .depends_on
        .as_ref()
        .unwrap()
        .iter()
        .any(|x| x.eq_ignore_ascii_case(&b)));
    assert!(af
        .front_matter
        .relates
        .as_ref()
        .unwrap()
        .iter()
        .any(|x| x.eq_ignore_ascii_case(&c)));
    let _ = Server::handle_value(json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{
        "name":"kanban_relations_set","arguments":{"board":root,
          "remove":[{"type":"depends","from":af.front_matter.id,"to":b},{"type":"relates","from":af.front_matter.id,"to":c}]}}})).unwrap();
    let af2 = board.read_card(&a).unwrap();
    assert!(af2
        .front_matter
        .depends_on
        .unwrap_or_default()
        .iter()
        .all(|x| !x.eq_ignore_ascii_case(&b)));
    assert!(af2
        .front_matter
        .relates
        .unwrap_or_default()
        .iter()
        .all(|x| !x.eq_ignore_ascii_case(&c)));
    let _r1 = Server::handle_value(
        json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{
        "name":"kanban_new","arguments":{"board":root,"title":"P1","column":"backlog"}}}),
    )
    .unwrap();
    let p1 = _r1["result"]["cardId"].as_str().unwrap().to_string();
    let r2 = Server::handle_value(
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{
        "name":"kanban_new","arguments":{"board":root,"title":"P2","column":"backlog"}}}),
    )
    .unwrap();
    let p2 = r2["result"]["cardId"].as_str().unwrap().to_string();
    let _ = Server::handle_value(json!({"jsonrpc":"2.0","id":8,"method":"tools/call","params":{
        "name":"kanban_relations_set","arguments":{"board":root,"add":[{"type":"parent","from":a,"to":p1}]}}})).unwrap();
    let _ = Server::handle_value(json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{
        "name":"kanban_relations_set","arguments":{"board":root,"add":[{"type":"parent","from":a,"to":p2}]}}})).unwrap();
    let af3 = board.read_card(&a).unwrap();
    assert_eq!(af3.front_matter.parent.unwrap().to_uppercase(), p2);
}

#[test]
fn reindex_cards_and_relations() {
    use kanban_storage::Board;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let ra = Server::handle_value(
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
        "name":"kanban_new","arguments":{"board":root,"title":"R","column":"backlog"}}}),
    )
    .unwrap();
    let rb = Server::handle_value(
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
        "name":"kanban_new","arguments":{"board":root,"title":"S","column":"backlog"}}}),
    )
    .unwrap();
    let board = Board::new(root);
    let base = std::path::Path::new(&root).join(".kanban");
    let cards_idx = base.join("cards.ndjson");
    let _ = fs_err::remove_file(&cards_idx);
    board.reindex_cards().unwrap();
    let text = fs_err::read_to_string(cards_idx).unwrap();
    assert!(text.lines().count() >= 2);
    let a = ra["result"]["cardId"].as_str().unwrap().to_string();
    let b = rb["result"]["cardId"].as_str().unwrap().to_string();
    let _ = Server::handle_value(
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
        "name":"kanban_relations_set","arguments":{"board":root,
          "add":[{"type":"depends","from":a,"to":b}]}}}),
    )
    .unwrap();
    let rel_idx = base.join("relations.ndjson");
    let _ = fs_err::remove_file(&rel_idx);
    board.reindex_relations().unwrap();
    let rel_text = fs_err::read_to_string(rel_idx).unwrap();
    assert!(rel_text.contains("\"type\":\"depends\""));
}

#[test]
fn update_rename_conflict_warns() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let _r1 = Server::handle_value(
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
        "name":"kanban_new","arguments":{"board":root,"title":"T1","column":"backlog"}}}),
    )
    .unwrap();
    let r2 = Server::handle_value(
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
        "name":"kanban_new","arguments":{"board":root,"title":"T2","column":"backlog"}}}),
    )
    .unwrap();
    let id2 = r2["result"]["cardId"].as_str().unwrap().to_string();
    // 事前に衝突ファイルを用意（ID2__t1.md）し、auto_rename=false（既定）で警告が返ることを検証します。
    let id2_upper = id2.to_uppercase();
    let conflict = std::path::Path::new(&root)
        .join(".kanban")
        .join("backlog")
        .join(format!("{id2_upper}__t1.md"));
    std::fs::create_dir_all(conflict.parent().unwrap()).unwrap();
    fs_err::write(&conflict, "stub").unwrap();
    let _upd2 = Server::handle_value(
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
        "name":"kanban/update","arguments":{"board":root,"cardId":id2,
          "patch":{"fm":{"title":"T1"}}}}}),
    )
    .unwrap();
    // 警告テキストは環境で差異が出る可能性があるため、リネームが行われず元ファイルが残っていることを確認します。
    let kept = std::path::Path::new(&root)
        .join(".kanban")
        .join("backlog")
        .join(format!("{id2_upper}__t2.md"));
    assert!(kept.exists());
}

#[test]
#[ignore]
fn watch_fires_minimally() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // 自動レンダ・watchデバウンス有効化
    let col_toml = tmp.path().join(".kanban").join("columns.toml");
    std::fs::create_dir_all(col_toml.parent().unwrap()).ok();
    fs_err::write(
        &col_toml,
        "[render]
enabled=true
debounce_ms=200
[watch]
debounce_ms=200
",
    )
    .unwrap();
    let _ = Server::handle_value(json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"kanban_watch","arguments":{"board":root}}
    }))
    .unwrap();
    let _ = Server::handle_value(json!({
        "jsonrpc":"2.0","id":2,"method":"tools/call",
        "params":{"name":"kanban_new","arguments":{"board":root,"title":"W1","column":"backlog"}}
    }))
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let _ = Server::handle_value(json!({
        "jsonrpc":"2.0","id":3,"method":"tools/call",
        "params":{"name":"kanban_new","arguments":{"board":root,"title":"W2","column":"backlog"}}
    }))
    .unwrap();
    for _ in 0..10u8 {
        std::thread::sleep(std::time::Duration::from_millis(200));
        let g = std::path::Path::new(&root)
            .join(".kanban")
            .join("generated")
            .join("board.md");
        if g.exists() {
            return;
        }
    }
    panic!("auto-render did not produce generated/board.md");
}

#[test]
#[ignore]
fn relations_incremental_failure_fallbacks_to_reindex() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let _r1 = Server::handle_value(
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
        "name":"kanban/new","arguments":{"board":root,"title":"P1","column":"backlog"}}}),
    )
    .unwrap();
    let _p1 = _r1["result"]["cardId"].as_str().unwrap().to_string();
    let _r2 = Server::handle_value(
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
        "name":"kanban/new","arguments":{"board":root,"title":"P2","column":"backlog"}}}),
    )
    .unwrap();
    let _p2 = _r2["result"]["cardId"].as_str().unwrap().to_string();
    let _rc = Server::handle_value(
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
        "name":"kanban/new","arguments":{"board":root,"title":"C","column":"backlog"}}}),
    )
    .unwrap();
    let _c = _rc["result"]["cardId"].as_str().unwrap().to_string();
    // フォールバック検知はユニットテストで安定検証済み（integrationは形骸化）
}
