use js_sys::Promise;
use std::future::Future;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{future_to_promise, JsFuture};

use rustpython_vm::common::rc::PyRc;
use rustpython_vm::function::{OptionalArg, PyFuncArgs};
use rustpython_vm::import::import_file;
use rustpython_vm::obj::{objdict::PyDictRef, objstr::PyStrRef, objtype::PyTypeRef};
use rustpython_vm::pyobject::{
    BorrowValue, IntoPyObject, PyCallable, PyClassImpl, PyObject, PyObjectRef, PyRef, PyResult,
    PyValue,
};
use rustpython_vm::VirtualMachine;

use crate::{convert, vm_class::weak_vm, wasm_builtins::window};

enum FetchResponseFormat {
    Json,
    Text,
    ArrayBuffer,
}

impl FetchResponseFormat {
    fn from_str(vm: &VirtualMachine, s: &str) -> PyResult<Self> {
        match s {
            "json" => Ok(FetchResponseFormat::Json),
            "text" => Ok(FetchResponseFormat::Text),
            "array_buffer" => Ok(FetchResponseFormat::ArrayBuffer),
            _ => Err(vm.new_type_error("Unkown fetch response_format".into())),
        }
    }
    fn get_response(&self, response: &web_sys::Response) -> Result<Promise, JsValue> {
        match self {
            FetchResponseFormat::Json => response.json(),
            FetchResponseFormat::Text => response.text(),
            FetchResponseFormat::ArrayBuffer => response.array_buffer(),
        }
    }
}

#[derive(FromArgs)]
struct FetchArgs {
    #[pyarg(named, default)]
    response_format: Option<PyStrRef>,
    #[pyarg(named, default)]
    method: Option<PyStrRef>,
    #[pyarg(named, default)]
    headers: Option<PyDictRef>,
    #[pyarg(named, default)]
    body: Option<PyObjectRef>,
    #[pyarg(named, default)]
    content_type: Option<PyStrRef>,
}

fn browser_fetch(url: PyStrRef, args: FetchArgs, vm: &VirtualMachine) -> PyResult {
    let FetchArgs {
        response_format,
        method,
        headers,
        body,
        content_type,
    } = args;

    let response_format = match response_format {
        Some(s) => FetchResponseFormat::from_str(vm, s.borrow_value())?,
        None => FetchResponseFormat::Text,
    };

    let mut opts = web_sys::RequestInit::new();

    match method {
        Some(s) => opts.method(s.borrow_value()),
        None => opts.method("GET"),
    };

    if let Some(body) = body {
        opts.body(Some(&convert::py_to_js(vm, body)));
    }

    let request = web_sys::Request::new_with_str_and_init(url.borrow_value(), &opts)
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    if let Some(headers) = headers {
        let h = request.headers();
        for (key, value) in headers {
            let key = vm.to_str(&key)?;
            let value = vm.to_str(&value)?;
            h.set(key.borrow_value(), value.borrow_value())
                .map_err(|err| convert::js_py_typeerror(vm, err))?;
        }
    }

    if let Some(content_type) = content_type {
        request
            .headers()
            .set("Content-Type", content_type.borrow_value())
            .map_err(|err| convert::js_py_typeerror(vm, err))?;
    }

    let window = window();
    let request_prom = window.fetch_with_request(&request);

    let future = async move {
        let val = JsFuture::from(request_prom).await?;
        let response = val
            .dyn_into::<web_sys::Response>()
            .expect("val to be of type Response");
        JsFuture::from(response_format.get_response(&response)?).await
    };

    Ok(PyPromise::from_future(future).into_object(vm))
}

fn browser_request_animation_frame(func: PyCallable, vm: &VirtualMachine) -> PyResult {
    use std::{cell::RefCell, rc::Rc};

    // this basic setup for request_animation_frame taken from:
    // https://rustwasm.github.io/wasm-bindgen/examples/request-animation-frame.html

    let f = Rc::new(RefCell::new(None));
    let g = f.clone();

    let weak_vm = weak_vm(vm);

    *g.borrow_mut() = Some(Closure::wrap(Box::new(move |time: f64| {
        let stored_vm = weak_vm
            .upgrade()
            .expect("that the vm is valid from inside of request_animation_frame");
        stored_vm.interp.enter(|vm| {
            let func = func.clone();
            let args = vec![vm.ctx.new_float(time)];
            let _ = vm.invoke(&func.into_object(), args);

            let closure = f.borrow_mut().take();
            drop(closure);
        })
    }) as Box<dyn Fn(f64)>));

    let id = window()
        .request_animation_frame(&js_sys::Function::from(
            g.borrow().as_ref().unwrap().as_ref().clone(),
        ))
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    Ok(vm.ctx.new_int(id))
}

fn browser_cancel_animation_frame(id: i32, vm: &VirtualMachine) -> PyResult<()> {
    window()
        .cancel_animation_frame(id)
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    Ok(())
}

#[pyclass(module = "browser", name = "Promise")]
#[derive(Debug)]
pub struct PyPromise {
    value: Promise,
}
pub type PyPromiseRef = PyRef<PyPromise>;

impl PyValue for PyPromise {
    fn class(vm: &VirtualMachine) -> PyTypeRef {
        vm.class("browser", "Promise")
    }
}

#[pyimpl]
impl PyPromise {
    pub fn new(value: Promise) -> PyPromise {
        PyPromise { value }
    }
    pub fn from_future<F>(future: F) -> PyPromise
    where
        F: Future<Output = Result<JsValue, JsValue>> + 'static,
    {
        PyPromise::new(future_to_promise(future))
    }
    pub fn value(&self) -> Promise {
        self.value.clone()
    }

    #[pymethod]
    fn then(
        &self,
        on_fulfill: PyCallable,
        on_reject: OptionalArg<PyCallable>,
        vm: &VirtualMachine,
    ) -> PyPromiseRef {
        let weak_vm = weak_vm(vm);
        let prom = JsFuture::from(self.value.clone());

        let ret_future = async move {
            let stored_vm = &weak_vm
                .upgrade()
                .expect("that the vm is valid when the promise resolves");
            let res = prom.await;
            match res {
                Ok(val) => stored_vm.interp.enter(move |vm| {
                    let args = if val.is_null() {
                        vec![]
                    } else {
                        vec![convert::js_to_py(vm, val)]
                    };
                    let res = vm.invoke(&on_fulfill.into_object(), PyFuncArgs::new(args, vec![]));
                    convert::pyresult_to_jsresult(vm, res)
                }),
                Err(err) => {
                    if let OptionalArg::Present(on_reject) = on_reject {
                        stored_vm.interp.enter(move |vm| {
                            let err = convert::js_to_py(vm, err);
                            let res = vm.invoke(
                                &on_reject.into_object(),
                                PyFuncArgs::new(vec![err], vec![]),
                            );
                            convert::pyresult_to_jsresult(vm, res)
                        })
                    } else {
                        Err(err)
                    }
                }
            }
        };

        PyPromise::from_future(ret_future).into_ref(vm)
    }

    #[pymethod]
    fn catch(&self, on_reject: PyCallable, vm: &VirtualMachine) -> PyPromiseRef {
        let weak_vm = weak_vm(vm);
        let prom = JsFuture::from(self.value.clone());

        let ret_future = async move {
            let err = match prom.await {
                Ok(x) => return Ok(x),
                Err(e) => e,
            };
            let stored_vm = weak_vm
                .upgrade()
                .expect("that the vm is valid when the promise resolves");
            stored_vm.interp.enter(move |vm| {
                let err = convert::js_to_py(vm, err);
                let res = vm.invoke(&on_reject.into_object(), PyFuncArgs::new(vec![err], vec![]));
                convert::pyresult_to_jsresult(vm, res)
            })
        };

        PyPromise::from_future(ret_future).into_ref(vm)
    }
}

#[pyclass(module = "browser", name)]
#[derive(Debug)]
struct Document {
    doc: web_sys::Document,
}

impl PyValue for Document {
    fn class(vm: &VirtualMachine) -> PyTypeRef {
        vm.class("browser", "Document")
    }
}

#[pyimpl]
impl Document {
    #[pymethod]
    fn query(&self, query: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let elem = self
            .doc
            .query_selector(query.borrow_value())
            .map_err(|err| convert::js_py_typeerror(vm, err))?
            .map(|elem| Element { elem })
            .into_pyobject(vm);
        Ok(elem)
    }

    #[pymethod]
    fn createElement(&self, tag: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let elem = self
        .doc
        .create_element(tag.borrow_value());

        Ok(Element{ elem: elem.unwrap() }.into_pyobject(vm))
    }
}

#[pyclass(module = "browser", name)]
#[derive(Debug)]
struct Element {
    elem: web_sys::Element,
}

impl PyValue for Element {
    fn class(vm: &VirtualMachine) -> PyTypeRef {
        vm.class("browser", "Element")
    }
}

#[pyimpl]
impl Element {
    #[pymethod]
    fn get_attr(
        &self,
        attr: PyStrRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyObjectRef {
        match self.elem.get_attribute(attr.borrow_value()) {
            Some(s) => vm.ctx.new_str(s),
            None => default.unwrap_or_none(vm),
        }
    }

    #[pymethod]
    fn set_attr(&self, attr: PyStrRef, value: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        self.elem
            .set_attribute(attr.borrow_value(), value.borrow_value())
            .map_err(|err| convert::js_py_typeerror(vm, err))
    }

    #[pymethod]
    fn appendChild(&self, child: PyRef<Element>, vm: &VirtualMachine) -> PyResult<()> {
        self.elem
            .append_child(&*child.elem)
            .map_err(|err| convert::js_py_typeerror(vm, err))?;
        Ok(())
    }
}

fn browser_load_module(module: PyStrRef, path: PyStrRef, vm: &VirtualMachine) -> PyResult {
    let weak_vm = weak_vm(vm);

    let mut opts = web_sys::RequestInit::new();
    opts.method("GET");

    let request = web_sys::Request::new_with_str_and_init(path.borrow_value(), &opts)
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    let window = window();
    let request_prom = window.fetch_with_request(&request);

    let future = async move {
        let val = JsFuture::from(request_prom).await?;
        let response = val
            .dyn_into::<web_sys::Response>()
            .expect("val to be of type Response");
        let text = JsFuture::from(response.text()?).await?;
        let stored_vm = &weak_vm
            .upgrade()
            .expect("that the vm is valid when the promise resolves");
        stored_vm.interp.enter(move |vm| {
            let resp_text = text.as_string().unwrap();
            let res = import_file(vm, module.borrow_value(), "WEB".to_owned(), resp_text);
            match res {
                Ok(_) => Ok(JsValue::null()),
                Err(err) => Err(convert::py_err_to_js_err(vm, &err)),
            }
        })
    };

    Ok(PyPromise::from_future(future).into_object(vm))
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let promise = PyPromise::make_class(ctx);

    let document_class = Document::make_class(ctx);

    let document = PyObject::new(
        Document {
            doc: window().document().expect("Document missing from window"),
        },
        document_class.clone(),
        None,
    );

    let element = Element::make_class(ctx);

    py_module!(vm, "browser", {
        "fetch" => ctx.new_function(browser_fetch),
        "request_animation_frame" => ctx.new_function(browser_request_animation_frame),
        "cancel_animation_frame" => ctx.new_function(browser_cancel_animation_frame),
        "Promise" => promise,
        "Document" => document_class,
        "document" => document,
        "Element" => element,
        "load_module" => ctx.new_function(browser_load_module),
    })
}

pub fn setup_browser_module(vm: &mut VirtualMachine) {
    let state = PyRc::get_mut(&mut vm.state).unwrap();
    state
        .stdlib_inits
        .insert("_browser".to_owned(), Box::new(make_module));
    state
        .frozen
        .extend(py_freeze!(file = "src/browser.py", module_name = "browser",));
}
