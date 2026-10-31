use armature_proc_macro::Validate;

#[derive(Validate)]
struct Signup {
    #[validate(regex = "[a-z")]
    username: String,
}

fn main() {}
