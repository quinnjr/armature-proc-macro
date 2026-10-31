use armature_proc_macro::Validate;

#[derive(Validate)]
struct Signup {
    #[validate(regex = "[a-z")]
    username: String,
}

// Parses, but is over the compiled-size limit the runtime enforces.
#[derive(Validate)]
struct Oversized {
    #[validate(regex = r"\w{300}")]
    word: String,
}

fn main() {}
