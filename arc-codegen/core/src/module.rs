use crate::error::ErrorKind::*;
use crate::error::*;
use crate::util::*;
use std::time::Instant;
use weld::ast::ScalarKind::I8;
use weld::ast::*;
use weld::data::*;
use weld::*;

pub type ModuleRun<A> = (A, u64);

pub struct Module {
    pub id: String,
    code: String,
    pub priority: i32,
    module: WeldModule,
    serialize_module: Option<WeldModule>,
    conf: WeldConf,
}

impl Module {
    pub fn new(id: String, code: String, priority: i32, threads: Option<i32>) -> Result<Module> {
        let mut conf = WeldConf::new();
        if let Some(t) = threads {
            let threads = format!("{}", t);
            conf.set("weld.threads", threads);
        }

        let module: WeldModule = WeldModule::compile(code.clone(), &conf).map_err(|e| {
            Error::new(CompilationError(e.message().to_string_lossy().into_owned()))
        })?;

        let serializer_str = serialize_module_fmt(code.clone())?;
        let serialize_module: WeldModule =
            WeldModule::compile(serializer_str, &conf).map_err(|e| {
                Error::new(CompilationError(e.message().to_string_lossy().into_owned()))
            })?;

        // Verify serialize_module actually outputs raw bytes
        if serialize_module.return_type() != ast::Type::Vector(Box::new(Type::Scalar(I8))) {
            return Err(Error::new(CompilationError(
                "Serialize module has to output Vec<i8>".to_string(),
            )));
        }

        Ok(Module {
            id,
            code,
            priority,
            module,
            serialize_module: Some(serialize_module),
            conf,
        })
    }

    pub fn conf(&self) -> &WeldConf {
        &self.conf
    }

    pub fn return_type(&self) -> weld::ast::Type {
        self.module.return_type()
    }

    pub fn param_types(&self) -> Vec<weld::ast::Type> {
        self.module.param_types()
    }

    pub fn serialize_input<I>(&mut self, ptr: &I, ctx: &mut WeldContext) -> Result<Vec<i8>> {
        let ref arg = WeldValue::new_from_data(ptr as *const _ as Data);
        let res = unsafe { self.serializer(ctx, arg) };
        if let Ok(raw) = res {
            let bytes = weld_to_raw(raw)?;
            Ok(bytes)
        } else {
            Err(Error::new(SerializationError(
                "Failed to serialize input for weld module".to_string(),
            )))
        }
    }

    pub fn raw_to_mat<O: Clone>(&self, bytes: &Vec<i8>, ctx: &mut WeldContext) -> Result<(O, u64)> {
        let input: WeldVec<i8> = WeldVec::from(bytes);
        self.run(&input, ctx)
    }

    pub fn raw_to_raw(&self, bytes: &Vec<i8>, ctx: &mut WeldContext) -> Result<Vec<i8>> {
        let input: WeldVec<i8> = WeldVec::from(bytes);
        let (result, _ns) = self.run(&input, ctx)?;
        weld_to_raw(result)
    }

    pub fn run<I, O: Clone>(&self, ptr: &I, ctx: &mut WeldContext) -> Result<(O, u64)> {
        let ref arg = WeldValue::new_from_data(ptr as *const _ as Data);
        let (result, time_ns) = unsafe {
            let (res, ns) = self.mat_runner(ctx, arg)?;
            let data = res.data() as *const O;
            ((*data).clone(), ns)
        };
        Ok((result, time_ns))
    }

    unsafe fn mat_runner(
        &self,
        ctx: &mut WeldContext,
        arg: &WeldValue,
    ) -> Result<(WeldValue, u64)> {
        let start = Instant::now();
        let res = self
            .module
            .run(ctx, arg)
            .map_err(|e| Error::new(ModuleRunError(e.message().to_string_lossy().into_owned())))?;
        let elapsed = start.elapsed();
        let ns: u64 = elapsed.as_secs() * 1_000_000_000 + u64::from(elapsed.subsec_nanos());
        Ok((res, ns))
    }

    unsafe fn serializer(&mut self, ctx: &mut WeldContext, arg: &WeldValue) -> Result<WeldVec<i8>> {
        if let Some(module) = &self.serialize_module {
            let res = module.run(ctx, arg).map_err(|e| {
                Error::new(ModuleRunError(e.message().to_string_lossy().into_owned()))
            })?;

            let data = res.data() as *const WeldVec<i8>;
            let cloned = (*data).clone();
            Ok(cloned)
        } else {
            Err(Error::new(ModuleRunError(
                "No serializer module found".to_string(),
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn addition_module() {
        let id = String::from("addition");
        let code = String::from("|x:i32| 40 + x");
        let priority = 0;
        let module = Module::new(id, code, priority, None).unwrap();
        let ref mut ctx = WeldContext::new(&module.conf).unwrap();
        assert_eq!(ctx.memory_usage(), 0);
        let arg: i32 = 10;
        let res: ModuleRun<i32> = module.run(&arg, ctx).unwrap();
        let data = res.0;
        let time_ns = res.1;
        assert_eq!(data, 50);
        assert!(ctx.memory_usage() > 0);
        assert!(time_ns > 0);
    }

    #[test]
    fn vec_len_module() {
        let id = String::from("vec_len");
        let code = String::from("|x:vec[i32]| len(x)");
        let priority = 0;
        let input_vec = [2, 3, 4, 2, 1];
        let input_data = WeldVec::from(&input_vec);
        let module = Module::new(id, code, priority, None).unwrap();
        let ref mut ctx = WeldContext::new(&module.conf).unwrap();
        let res: ModuleRun<i32> = module.run(&input_data, ctx).unwrap();
        let data = res.0;
        assert_eq!(data, 5);
    }

    #[test]
    fn vec_filter_module() {
        let id = String::from("filter_vec");
        let code = String::from("|v: vec[i32]| filter(v, |a:i32| a > 2)");
        let priority = 0;
        let input_vec = [2, 3, 4, 2, 1];
        let input_data = WeldVec::from(&input_vec);
        let module = Module::new(id, code, priority, None).unwrap();
        let ref mut ctx = WeldContext::new(&module.conf).unwrap();
        let result: ModuleRun<WeldVec<i32>> = module.run(&input_data, ctx).unwrap();
        assert_eq!(result.0.len, 2);
        // Run the module again with the returned Vec
        let result: ModuleRun<WeldVec<i32>> = module.run(&result, ctx).unwrap();
        // Should equal in the same
        assert_eq!(result.0.len, 2);
    }

    #[test]
    fn full_module_pass() {
        #[repr(C)]
        struct MyArgs {
            x: WeldVec<i32>,
            y: WeldVec<i32>,
        }

        let id = String::from("filter_vec");
        // NOTE: Example taken from the weld tests!
        let x = vec![1, 2, 3, 4];
        let y = vec![5, 6, 7, 8];
        let code = "|x:vec[i32], y:vec[i32]| map(zip(x,y), |e| e.$0 + e.$1)";
        let priority = 0;
        let ref input_data = MyArgs {
            x: WeldVec::from(&x),
            y: WeldVec::from(&y),
        };
        let mut module = Module::new(id, code.clone().to_string(), priority, None).unwrap();
        let ref mut ctx = WeldContext::new(&module.conf).unwrap();

        // Serialize our input data
        let serialized_input: Vec<i8> = module.serialize_input(input_data, ctx).unwrap();

        // Generate a raw module of the original code
        // and run it using the serialized input
        let raw_module_code = generate_raw_module(code.to_string(), false).unwrap();
        let priority = 0;
        let new_id = String::from("raw");
        let module = Module::new(new_id, raw_module_code, priority, None).unwrap();
        let ref mut ctx = WeldContext::new(&module.conf).unwrap();
        let r: ModuleRun<WeldVec<i32>> = module.raw_to_mat(&serialized_input, ctx).unwrap();
        let result = r.0;
        let expected = vec![6, 8, 10, 12];
        for i in 0..(result.len as isize) {
            assert_eq!(unsafe { *result.data.offset(i) }, expected[i as usize])
        }
    }

    #[test]
    fn multi_thread_module() {
        let id = String::from("addition");
        let code = String::from("|x:i32| 40 + x");
        let priority = 0;
        let module = Arc::new(Module::new(id, code, priority, None).unwrap());
        let num_threads = 4;
        let mut threads = vec![];
        for _ in 0..num_threads {
            let module = Arc::clone(&module);
            threads.push(thread::spawn(move || {
                let ref mut ctx = WeldContext::new(&module.conf()).unwrap();
                let input: i32 = 10;
                let result: ModuleRun<i32> = module.run(&input, ctx).unwrap();
                assert_eq!(result.0, 50);
            }))
        }
        for t in threads {
            t.join().unwrap();
        }
    }
}