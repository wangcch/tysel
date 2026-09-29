#[cfg(target_os = "linux")]
#[test]
fn process_tree_includes_children_spawned_by_other_threads() {
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use tysel_bench_compare::{process_memory_kb, process_tree_memory, process_tree_pids};

    let (ready_tx, ready_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let mut child = Command::new("sleep").arg("30").stdout(Stdio::null()).spawn().unwrap();
        ready_tx.send(child.id()).unwrap();
        let _ = stop_rx.recv();
        let _ = child.kill();
        child.wait().unwrap();
    });
    let child = ready_rx.recv().unwrap();
    let root = std::process::id();
    let pids = process_tree_pids(root).unwrap();
    let memory = process_tree_memory(root).unwrap();
    let child_memory = process_memory_kb(child).unwrap();
    stop_tx.send(()).unwrap();
    thread.join().unwrap();
    assert!(pids.contains(&child), "thread-owned worker must not be omitted");
    assert!(memory.process_count >= 2);
    assert_eq!(memory.kind, "pss");
    assert!(memory.value_kb >= child_memory.0);
}
