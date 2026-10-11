// Read-only access to an actual external-scheduler root before job consumption.
impl Interpreter {
    /// Inspect the genuine queueMicrotask callback retained by this token.
    ///
    /// Promise reactions and module-import jobs return `None`: their rejection
    /// handling remains owned by their existing job algorithms. Inspection does
    /// not consume the token, invoke JavaScript, or follow a callable's target.
    /// The embedding can read the returned object's associated creation realm
    /// before `invoke_one_job` removes the scheduler root.
    pub fn binding_scheduled_microtask_callback(
        &self, token: SchedulingToken,
    ) -> Result<Option<JsValue>, Throw> {
        let external = self.external_scheduling.as_ref()
            .ok_or_else(|| Throw::type_error("external scheduler is not enabled"))?;
        if token.owner != external.owner {
            return Err(Throw::type_error("foreign scheduling token"));
        }
        match external.tasks.get(&token.sequence) {
            Some(ScheduledTask::Job(Job::Microtask(callback))) => Ok(Some(callback.clone())),
            Some(ScheduledTask::Job(Job::Promise(_) | Job::ModuleImport(_))) => Ok(None),
            _ => Err(Throw::type_error("unknown or wrong-kind scheduling token")),
        }
    }
}

#[cfg(test)]
mod scheduled_microtask_inspection_tests {
    use super::*;

    fn setup(backend: ExecutionBackend) -> Interpreter {
        let mut i=Interpreter::new_with_external_scheduler(64).unwrap();
        i.force_ast_callables=backend==ExecutionBackend::TreeWalk; i
    }
    fn tokens(i: &mut Interpreter) -> Vec<SchedulingToken> {
        i.take_scheduling_requests().into_iter().filter_map(|request| match request {
            SchedulingRequest::Microtask{job_token}=>Some(job_token),_=>None,
        }).collect()
    }
    fn same(left: &JsValue,right: &JsValue) {
        let (JsValue::Object(left),JsValue::Object(right))=(left,right) else{panic!("objects");};
        assert!(Rc::ptr_eq(&left.0,&right.0));
    }

    #[test]
    fn inspection_retains_the_actual_callback_without_execution_or_consumption() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=setup(backend);
            let callback=i.run_script_with_backend("var calls=0;function cb(){calls++;}cb",backend).unwrap();
            i.queue_microtask(callback.clone()).unwrap();let token=tokens(&mut i)[0];
            same(&callback,&i.binding_scheduled_microtask_callback(token).unwrap().unwrap());
            same(&callback,&i.binding_scheduled_microtask_callback(token).unwrap().unwrap());
            assert_eq!(i.run_script("calls").unwrap(),JsValue::num(0.0));
            i.invoke_one_job(token).unwrap();
            assert_eq!(i.run_script("calls").unwrap(),JsValue::num(1.0));
            assert!(i.binding_scheduled_microtask_callback(token).is_err());
        }
    }

    #[test]
    fn callback_associated_realm_is_its_actual_object_realm_not_target_or_current() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=setup(backend);let parent=i.binding_current_realm();
            let global=i.binding_create_object(None).unwrap();let child=i.binding_create_realm(global).unwrap();
            let callback=i.binding_with_realm(child,|i|i.run_script_with_backend("(function cb(){return 1;})",backend)).unwrap();
            let JsValue::Object(object)=&callback else{panic!("function");};
            let body=object.borrow().func.as_ref().unwrap().body.clone();
            assert!(matches!((backend,body),(ExecutionBackend::TreeWalk,FunctionBody::Block(_))|(ExecutionBackend::Bytecode,FunctionBody::Bytecode(_))));
            i.queue_microtask(callback.clone()).unwrap();let token=tokens(&mut i)[0];
            let inspected=i.binding_scheduled_microtask_callback(token).unwrap().unwrap();
            same(&callback,&inspected);assert_eq!(i.binding_object_creation_realm(&inspected).unwrap(),child);
            assert_eq!(i.binding_current_realm(),parent);i.invoke_one_job(token).unwrap();
            for source in ["(class NativeClass{})","Array.prototype.push"] {
                let callback=i.binding_with_realm(child,|i|i.run_script_with_backend(source,backend)).unwrap();
                i.queue_microtask(callback.clone()).unwrap();let token=tokens(&mut i)[0];
                let inspected=i.binding_scheduled_microtask_callback(token).unwrap().unwrap();same(&callback,&inspected);
                assert_eq!(i.binding_object_creation_realm(&inspected).unwrap(),child);
                assert!(i.invoke_one_job(token).is_err());
            }
        }
    }

    #[test]
    fn revoked_proxy_and_bound_callbacks_do_not_trigger_target_or_authored_getters() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=setup(backend);let parent=i.binding_current_realm();
            let global=i.binding_create_object(None).unwrap();let child=i.binding_create_realm(global).unwrap();
            let target=i.binding_with_realm(child,|i|i.run_script_with_backend("(function target(){})",backend)).unwrap();
            i.global_env.declare_var("target",target).unwrap();
            for source in ["Function.prototype.bind.call(target,null)","var rev=Proxy.revocable(target,{get(){throw 99;}});rev.proxy"] {
                let callback=i.run_script_with_backend(source,backend).unwrap();
                i.queue_microtask(callback.clone()).unwrap();let token=tokens(&mut i)[0];
                if source.contains("rev="){i.run_script("rev.revoke()").unwrap();}
                let inspected=i.binding_scheduled_microtask_callback(token).unwrap().unwrap();same(&callback,&inspected);
                assert_eq!(i.binding_object_creation_realm(&inspected).unwrap(),parent);
                if source.contains("rev="){assert!(i.invoke_one_job(token).is_err());}else{i.invoke_one_job(token).unwrap();}
            }
        }
    }

    #[test]
    fn invalid_owner_and_kind_are_rejected_without_consuming_real_work() {
        let mut i=setup(ExecutionBackend::Bytecode);let callback=i.run_script("()=>{}").unwrap();
        i.queue_microtask(callback.clone()).unwrap();let token=tokens(&mut i)[0];
        let mut other=setup(ExecutionBackend::Bytecode);
        assert!(other.binding_scheduled_microtask_callback(token).is_err());
        assert!(Interpreter::new().binding_scheduled_microtask_callback(token).is_err());
        let forged=SchedulingToken{owner:token.owner,sequence:token.sequence+100};
        assert!(i.binding_scheduled_microtask_callback(forged).is_err());
        i.run_script("var timerCalled=false;setTimeout(()=>{timerCalled=true;},0)").unwrap();
        let timer=i.take_scheduling_requests().into_iter().find_map(|request|match request{
            SchedulingRequest::Timer{callback_token,..}=>Some(callback_token),_=>None,
        }).unwrap();
        assert!(i.binding_scheduled_microtask_callback(timer).is_err());
        i.invoke_one_callback(timer,JsValue::Undefined,None).unwrap();
        assert_eq!(i.run_script("timerCalled").unwrap(),JsValue::Bool(true));
        same(&callback,&i.binding_scheduled_microtask_callback(token).unwrap().unwrap());i.invoke_one_job(token).unwrap();
    }

    #[test]
    fn promise_reactions_and_genuine_module_failure_jobs_keep_existing_rejection_paths() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=setup(backend);
            i.run_script_with_backend("var reacted=false;Promise.resolve(1).then(()=>{reacted=true;});",backend).unwrap();
            let token=tokens(&mut i)[0];assert!(i.binding_scheduled_microtask_callback(token).unwrap().is_none());
            i.invoke_one_job(token).unwrap();assert_eq!(i.run_script("reacted").unwrap(),JsValue::Bool(true));
            i.run_script_with_backend("var rejected=false;import('absent').catch(()=>{rejected=true;});",backend).unwrap();
            let imports=tokens(&mut i);assert_eq!(imports.len(),1);
            assert!(i.binding_scheduled_microtask_callback(imports[0]).unwrap().is_none());i.invoke_one_job(imports[0]).unwrap();
            // Any rejection reaction scheduled by the import job is also a native job.
            for reaction in tokens(&mut i) {
                assert!(i.binding_scheduled_microtask_callback(reaction).unwrap().is_none());i.invoke_one_job(reaction).unwrap();
            }
            assert_eq!(i.run_script("rejected").unwrap(),JsValue::Bool(true));
        }
    }

    #[test]
    fn original_throw_identity_and_remaining_queue_are_preserved() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=setup(backend);
            let error=i.run_script_with_backend("var error={marker:1};var after=false;queueMicrotask(()=>{throw error;});queueMicrotask(()=>{after=true;});error",backend).unwrap();
            let jobs=tokens(&mut i);assert_eq!(jobs.len(),2);
            let callback=i.binding_scheduled_microtask_callback(jobs[0]).unwrap().unwrap();
            let realm=i.binding_object_creation_realm(&callback).unwrap();
            let thrown=i.invoke_one_job(jobs[0]).unwrap_err();same(&error,&thrown.0);
            assert_eq!(realm,i.binding_current_realm());
            assert!(i.binding_scheduled_microtask_callback(jobs[0]).is_err());
            assert!(i.binding_scheduled_microtask_callback(jobs[1]).unwrap().is_some());i.invoke_one_job(jobs[1]).unwrap();
            assert_eq!(i.run_script("after").unwrap(),JsValue::Bool(true));
        }
    }
}
