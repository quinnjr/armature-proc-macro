use armature_proc_macro::Validate;

// The override is honoured: a path that does not exist fails to resolve.
#[derive(Validate)]
#[validate(crate = "crate::does_not_exist")]
struct Login {
    #[validate(required)]
    user: String,
}

fn main() {}
