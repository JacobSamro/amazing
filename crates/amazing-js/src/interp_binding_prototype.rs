//! Native embedding adapter for the existing [[SetPrototypeOf]] implementation.
use super::*;

impl Interpreter {
    /// Apply the actual object's [[SetPrototypeOf]] without looking up mutable
    /// JavaScript Reflect/Object methods. A rejected change returns false;
    /// abrupt completions, including Proxy throws, retain their original value.
    pub fn binding_set_prototype_of(&mut self, target: &JsValue, prototype: &JsValue) -> Result<bool, Throw> {
        let JsValue::Object(target) = target else {
            return Err(Throw::type_error("target must be an object"));
        };
        let prototype = match prototype {
            JsValue::Object(object) => Some(object.clone()),
            JsValue::Null => None,
            _ => return Err(Throw::type_error("prototype must be an object or null")),
        };
        self.default_set_prototype(target, &prototype)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pair(i: &mut Interpreter, source: &str) -> (JsValue, JsValue) {
        let values=i.run_script(source).unwrap();
        let target=i.binding_get_property(&values,"0").unwrap();
        let prototype=i.binding_get_property(&values,"1").unwrap();(target,prototype)
    }
    #[test]
    fn native_changes_null_same_and_foreign_prototypes_with_no_reflect_lookup() {
        let mut i=Interpreter::new();let (target,prototype)=pair(&mut i,"var target={},prototype={};[target,prototype]");
        i.run_script("Object.setPrototypeOf=function(){throw 'poison'};Object.defineProperty(Reflect,'setPrototypeOf',{get(){throw 'poison'}})").unwrap();
        assert!(i.binding_set_prototype_of(&target,&prototype).unwrap());
        assert!(i.binding_set_prototype_of(&target,&prototype).unwrap());
        assert_eq!(i.run_script("Object.getPrototypeOf(target)===prototype").unwrap(),JsValue::Bool(true));
        assert!(i.binding_set_prototype_of(&target,&JsValue::Null).unwrap());
        assert_eq!(i.run_script("Object.getPrototypeOf(target)===null").unwrap(),JsValue::Bool(true));
        let global=i.binding_create_object(None).unwrap();let child=i.binding_create_realm(global).unwrap();
        let foreign=i.binding_with_realm(child,|i|i.run_script("Object.prototype")).unwrap();
        assert!(i.binding_set_prototype_of(&target,&foreign).unwrap());
        let JsValue::Object(target)=target else {panic!("target")};
        assert!(i.binding_same_value(&JsValue::Object(target.borrow().proto.clone().unwrap()),&foreign));
    }
    #[test]
    fn native_rejections_preserve_extensibility_immutable_prototypes_and_cycles() {
        let mut i=Interpreter::new();
        for source in ["[Object.preventExtensions({}),{}]","[Object.prototype,{}]","var self={};[self,self]","var a={},b=Object.create(a);[a,b]"] {
            let (target,prototype)=pair(&mut i,source);
            assert!(!i.binding_set_prototype_of(&target,&prototype).unwrap());
        }
        let (target,prototype)=pair(&mut i,"var p={},fixed=Object.create(p);Object.preventExtensions(fixed);[fixed,p]");
        assert!(i.binding_set_prototype_of(&target,&prototype).unwrap());
        assert!(i.binding_set_prototype_of(&i.binding_object_prototype(),&JsValue::Null).unwrap());
    }
    #[test]
    fn proxy_traps_receive_real_operands_and_false_remains_false() {
        let mut i=Interpreter::new();
        for answer in [true,false] {
            let source=format!("var seen=0,base={{}},proto={{}},proxy=new Proxy(base,{{setPrototypeOf(t,p){{if(t!==base||p!==proto)throw 'operands';seen++;return {answer}}}}});[proxy,proto]");
            let (target,prototype)=pair(&mut i,&source);
            assert_eq!(i.binding_set_prototype_of(&target,&prototype).unwrap(),answer);
            assert_eq!(i.run_script("seen").unwrap(),JsValue::num(1.0));
        }
    }
    #[test]
    fn proxy_abrupt_revocation_and_nonextensible_invariants_are_unmodified() {
        let mut i=Interpreter::new();
        let (target,prototype)=pair(&mut i,"var marker={};[new Proxy({},{setPrototypeOf(){throw marker}}),{}]");
        let marker=i.run_script("marker").unwrap();let error=i.binding_set_prototype_of(&target,&prototype).unwrap_err();
        assert!(i.binding_same_value(&marker,&error.0));
        for source in ["var r=Proxy.revocable({},{});r.revoke();[r.proxy,{}]","[new Proxy(Object.preventExtensions({}),{setPrototypeOf(){return true}}),{}]"] {
            let (target,prototype)=pair(&mut i,source);assert!(i.binding_set_prototype_of(&target,&prototype).is_err());
        }
    }
    #[test]
    fn primitive_inputs_reject_without_coercion_or_author_reads() {
        let mut i=Interpreter::new();let object=i.binding_create_object(None).unwrap();
        for primitive in [JsValue::Undefined,JsValue::Null,JsValue::Bool(false),JsValue::num(0.0),JsValue::str("x")] {
            assert!(i.binding_set_prototype_of(&primitive,&object).is_err());
            if primitive!=JsValue::Null {assert!(i.binding_set_prototype_of(&object,&primitive).is_err());}
        }
        let prototype=i.run_script("var coerced=0;({[Symbol.toPrimitive](){coerced++;throw 'coercion'}})").unwrap();
        assert!(i.binding_set_prototype_of(&object,&prototype).unwrap());
        assert_eq!(i.run_script("coerced").unwrap(),JsValue::num(0.0));
    }
}
