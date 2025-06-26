use control_plane::server::{create as create_cp, delete as delete_cp};

pub fn create(name: String) {
    create_cp(name);
}
pub fn delete(name: String) {
    delete_cp(name);
}
