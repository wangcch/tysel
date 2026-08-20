//! Read-only grants are rejected before a database connection is opened.

use tysel_cap_postgres::{configure, exec};

#[tokio::test]
async fn exec_is_rejected_before_connect() {
    configure(Some("postgres://invalid".into()), true);
    let err = exec("CREATE TABLE must_not_exist (id INTEGER)", "[]").await.unwrap_err();
    assert_eq!(err, "postgres connection is read-only");
}
