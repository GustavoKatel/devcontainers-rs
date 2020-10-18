use bollard::service::Mount;

use super::mount_from_str::*;

#[test]
fn test_from_comma() {
    let m = Mount::parse_from_str("/home/user/.config/nvim:/root/.config/nvim").unwrap();

    assert_eq!(m.source, Some("/home/user/.config/nvim".to_string()));
    assert_eq!(m.target, Some("/root/.config/nvim".to_string()));
}
