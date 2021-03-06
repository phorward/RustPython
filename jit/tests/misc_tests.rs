use rustpython_jit::{AbiValue, JitArgumentError};

// TODO currently broken
// #[test]
// fn test_no_return_value() {
//     let func = jit_function! { func() => r##"
//         def func():
//             pass
//     "## };
//
//     assert_eq!(func(), Ok(()));
// }

#[test]
fn test_invoke() {
    let func = jit_function! { func => r##"
        def func(a: int, b: float):
            return 1
    "## };

    assert_eq!(
        func.invoke(&[AbiValue::Int(1)]),
        Err(JitArgumentError::WrongNumberOfArguments)
    );
    assert_eq!(
        func.invoke(&[AbiValue::Int(1), AbiValue::Float(2.0), AbiValue::Int(0)]),
        Err(JitArgumentError::WrongNumberOfArguments)
    );
    assert_eq!(
        func.invoke(&[AbiValue::Int(1), AbiValue::Int(1)]),
        Err(JitArgumentError::ArgumentTypeMismatch)
    );
    assert_eq!(
        func.invoke(&[AbiValue::Int(1), AbiValue::Float(2.0)]),
        Ok(Some(AbiValue::Int(1)))
    );
}

#[test]
fn test_args_builder() {
    let func = jit_function! { func=> r##"
        def func(a: int, b: float):
            return 1
    "## };

    let mut args_builder = func.args_builder();
    assert_eq!(args_builder.set(0, AbiValue::Int(1)), Ok(()));
    assert!(args_builder.is_set(0));
    assert!(!args_builder.is_set(1));
    assert_eq!(
        args_builder.set(1, AbiValue::Int(1)),
        Err(JitArgumentError::ArgumentTypeMismatch)
    );
    assert!(args_builder.is_set(0));
    assert!(!args_builder.is_set(1));
    assert!(args_builder.into_args().is_none());

    let mut args_builder = func.args_builder();
    assert_eq!(args_builder.set(0, AbiValue::Int(1)), Ok(()));
    assert_eq!(args_builder.set(1, AbiValue::Float(1.0)), Ok(()));
    assert!(args_builder.is_set(0));
    assert!(args_builder.is_set(1));

    let args = args_builder.into_args();
    assert!(args.is_some());
    assert_eq!(args.unwrap().invoke(), Some(AbiValue::Int(1)));
}
