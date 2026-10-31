use armature_proc_macro::Validate;

// `required` on a generic `Option<T>` needs `T: AsRef<str>`: without the
// bound it fails to compile instead of silently skipping the emptiness check.
#[derive(Validate)]
struct Named<T> {
    #[validate(required)]
    name: Option<T>,
}

// Likewise a generic wrapper the derive cannot see through: it must itself
// be `AsRef<str>`, not silently mean "is `Some`".
struct Wrapper<T>(T);

#[derive(Validate)]
struct Wrapped<T> {
    #[validate(required)]
    name: Option<Wrapper<T>>,
}

// A type macro nested in another type may hide the parameter: fail closed.
macro_rules! id {
    ($t:ty) => {
        $t
    };
}

struct N<T>(T);

#[derive(Validate)]
struct HiddenByMacro<T> {
    #[validate(required)]
    name: Option<N<id!(T)>>,
}

fn main() {}
