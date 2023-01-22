use std::path::PathBuf;
use tokio;

use crate::project::*;

#[tokio::test]
async fn test_new() {
    //let dc = Project::new(Some(PathBuf::from("/tmp")), None).unwrap();
    //assert_eq!(dc.path.to_str().unwrap(), "/tmp");

    let dc = Project::new(ProjectOpts::default()).unwrap();
    let dir = std::env::current_dir().unwrap();
    assert_eq!(dc.path.to_str().unwrap(), dir.to_str().unwrap())
}

#[tokio::test]
async fn test_validate_valid() {
    let mut dir = std::env::current_dir().unwrap();
    dir.push("test_files");
    dir.push("docker-compose");
    let mut dc = Project::new(ProjectOpts {
        path: Some(dir),
        ..ProjectOpts::default()
    })
    .unwrap();
    dc.load().await.unwrap();
}

#[tokio::test]
#[should_panic]
async fn test_validate_does_not_exist() {
    let dir = PathBuf::from("abc");
    let _ = Project::new(ProjectOpts {
        path: Some(dir),
        ..ProjectOpts::default()
    })
    .unwrap();
}

#[tokio::test]
async fn test_validate_invalid() {
    let mut dir = std::env::current_dir().unwrap();
    dir.push("test_files");
    dir.push("invalid");
    let mut dc = Project::new(ProjectOpts {
        path: Some(dir),
        ..ProjectOpts::default()
    })
    .unwrap();

    match dc.load().await {
        Err(_) => {}
        _ => panic!("Expected error"),
    };
}
