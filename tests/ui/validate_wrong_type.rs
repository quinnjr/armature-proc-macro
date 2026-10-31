use armature_proc_macro::Validate;

// A rule on a field of the wrong type is reported at the rule.
#[derive(Validate)]
struct WrongType {
    #[validate(email)]
    a: u32,
}

fn main() {}
