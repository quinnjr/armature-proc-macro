use armature_proc_macro::Validate;

// A rule on a field of the wrong type is reported at the rule.
#[derive(Validate)]
struct WrongType {
    #[validate(email)]
    a: u32,
    #[validate(range(min = 1))]
    b: Unit,
    #[validate]
    c: u32,
}

struct Unit;

fn main() {}
