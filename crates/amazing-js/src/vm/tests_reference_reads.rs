use super::*;
use super::super::FrameOutcome;
use crate::interp::Interpreter;

#[test]
fn ordinary_member_bytecode_checks_base_after_raw_expression_before_tokey() {
    let ast=crate::parse_script("out=base[raw()]").unwrap();
    let code=compile(&ast,false).unwrap();
    let instructions=&code.code;
    let coerce=instructions.iter().position(|op|matches!(op,Instruction::RequireObjectCoercible(_))).unwrap();
    let key=instructions.iter().position(|op|matches!(op,Instruction::ToKey(_))).unwrap();
    let get=instructions.iter().position(|op|matches!(op,Instruction::GetMember(..))).unwrap();
    assert!(coerce<key&&key<get);
    assert!(!instructions.iter().any(|op|matches!(op,Instruction::TreeEval(..))));
}

#[test]
fn ordinary_read_roots_survive_instruction_quantums_and_key_mutation() {
    let mut i=Interpreter::new();
    i.run_script(r#"
        var log=[],old={a:7,b:8},base=old,out;
        var key={name:'a',toString(){log.push('key');return this.name}};
        function getBase(){log.push('base');return base}
        function raw(){log.push('raw');return key}
    "#).unwrap();
    let code=compile(&crate::parse_script("out=getBase()[raw()];out").unwrap(),false).unwrap();
    let env=i.global_env.clone();let mut frame=code.start_frame(&mut i,env,1000).unwrap();
    let mut base_changed=false;let mut key_changed=false;let mut slices=0;
    loop {
        match code.resume_frame(&mut i,&mut frame,1).unwrap() {
            FrameOutcome::Suspended=>{
                slices+=1;
                if !base_changed&&i.run_script("log.join(',')==='base'").unwrap()==JsValue::Bool(true){
                    i.run_script("base={a:99,b:100}").unwrap();base_changed=true;
                }
                if !key_changed&&i.run_script("log.join(',')==='base,raw'").unwrap()==JsValue::Bool(true){
                    i.run_script("key.name='b'").unwrap();key_changed=true;
                }
            }
            FrameOutcome::Completed(value)=>{assert_eq!(value,JsValue::num(8.0));break;}
            _=>panic!("unexpected read outcome"),
        }
    }
    assert!(base_changed&&key_changed&&slices>4);
    assert_eq!(i.run_script("out===8&&log.join(',')==='base,raw,key'").unwrap(),JsValue::Bool(true));
}
