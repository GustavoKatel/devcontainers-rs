use super::devcontainer::*;

#[test]
#[should_panic]
fn test_no_valid_source() {
    let dc = DevContainer::default();
    dc.validate().unwrap()
}

#[test]
#[should_panic]
fn test_multiple_sources() {
    let dc = DevContainer {
        image: Some("myimage".to_string()),
        docker_compose_file: Some(DockerComposeFile::File("docker-compose.yaml".to_string())),
        ..Default::default()
    };
    dc.validate().unwrap()
}

#[test]
#[should_panic]
fn test_invalid_image() {
    let dc = DevContainer {
        image: Some("".to_string()),
        ..Default::default()
    };
    dc.validate().unwrap()
}

#[test]
#[should_panic]
fn test_invalid_docker_compose() {
    let dc = DevContainer {
        docker_compose_file: Some(DockerComposeFile::File("".to_string())),
        ..Default::default()
    };
    dc.validate().unwrap()
}

#[test]
#[should_panic]
fn test_invalid_docker_compose_arr() {
    let dc = DevContainer {
        docker_compose_file: Some(DockerComposeFile::Files(vec![])),
        ..Default::default()
    };
    dc.validate().unwrap()
}

#[test]
#[should_panic]
fn test_invalid_docker_compose_service() {
    let dc = DevContainer {
        docker_compose_file: Some(DockerComposeFile::File("docker-compose.yaml".to_string())),
        service: None,
        ..Default::default()
    };
    dc.validate().unwrap()
}

#[test]
#[should_panic]
fn test_invalid_docker_compose_service_empty_string() {
    let dc = DevContainer {
        docker_compose_file: Some(DockerComposeFile::File("docker-compose.yaml".to_string())),
        service: Some("".to_string()),
        ..Default::default()
    };
    dc.validate().unwrap()
}
