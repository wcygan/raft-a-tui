use raft::prelude::*;
use raft::storage::Storage;
use raft_core::storage::RaftStorage;

#[test]
fn test_new_storage_has_zero_applied_index() {
    let storage = RaftStorage::new();
    assert_eq!(storage.applied_index(), 0);
}

#[test]
fn test_set_and_get_applied_index() {
    let storage = RaftStorage::new();

    storage.set_applied_index(5);
    assert_eq!(storage.applied_index(), 5);

    storage.set_applied_index(100);
    assert_eq!(storage.applied_index(), 100);
}

#[test]
fn test_applied_index_shared_across_clones() {
    let storage1 = RaftStorage::new();
    storage1.set_applied_index(10);

    // Clone shares the same Arc<Mutex<u64>>
    let storage2 = storage1.clone();
    assert_eq!(storage2.applied_index(), 10);

    // Updating one affects the other
    storage2.set_applied_index(20);
    assert_eq!(storage1.applied_index(), 20);
    assert_eq!(storage2.applied_index(), 20);
}

#[test]
fn test_new_with_conf_state() {
    let conf_state = ConfState {
        voters: vec![1, 2, 3],
        ..Default::default()
    };

    let storage = RaftStorage::new_with_conf_state(conf_state.clone());

    // Applied index should still be 0
    assert_eq!(storage.applied_index(), 0);

    // Verify ConfState was set correctly
    let raft_state = storage.initial_state().unwrap();
    assert_eq!(raft_state.conf_state.voters, vec![1, 2, 3]);
}

#[test]
fn test_storage_trait_delegation_first_index() {
    let storage = RaftStorage::new();

    // MemStorage starts with first_index = last_index + 1
    // For empty storage, this should be consistent
    let first = storage.first_index().unwrap();
    let last = storage.last_index().unwrap();

    assert!(first > 0, "first_index should be > 0");
    assert_eq!(
        first,
        last + 1,
        "Empty storage: first_index should be last_index + 1"
    );
}

#[test]
fn test_storage_trait_delegation_initial_state() {
    let storage = RaftStorage::new();
    let state = storage.initial_state().unwrap();

    // Default HardState has term=0, vote=0, commit=0
    assert_eq!(state.hard_state.term, 0);
    assert_eq!(state.hard_state.vote, 0);
    assert_eq!(state.hard_state.commit, 0);
}

#[test]
fn test_applied_index_independence_from_commit_index() {
    let storage = RaftStorage::new();

    // Applied index is independent from Raft's commit index
    storage.set_applied_index(50);

    let state = storage.initial_state().unwrap();
    assert_eq!(state.hard_state.commit, 0, "Commit index from MemStorage");
    assert_eq!(
        storage.applied_index(),
        50,
        "Applied index tracked separately"
    );
}
