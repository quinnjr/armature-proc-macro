use armature_proc_macro::Validate;

// `required` on a generic `Option<T>` needs `T: AsRef<str>`: without the
// bound it fails to compile instead of silently skipping the emptiness check.
#[derive(Validate)]
struct Named<T> {
    #[validate(required)]
    name: Option<T>,
}

fn main() {}
