//! ElementInternals / form-associated custom elements through the installed runtime.
use super::*;
use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};
fn scheduler(markup: &str) -> DriverScheduler {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse(markup), native_forwarded: false,
    }));
    DriverScheduler::new(host, JS_CAPACITY).unwrap()
}
#[test]
fn attach_internals_works_for_create_new_and_upgrade() {
    let mut driver = scheduler("<body><form id=f><x-a id=p></x-a></form></body>");
    let result = driver.run_script(r#"
        let log=[];
        customElements.define('x-a', class extends HTMLElement {
            static get formAssociated(){return true;}
            constructor(){ super(); try { this.i = this.attachInternals(); } catch(e) { log.push('ctor:'+e); } }
        });
        let c=document.createElement('x-a'); log.push('create:'+(c.i!==undefined));
        let n=new (customElements.get('x-a')); log.push('new:'+(n.i!==undefined));
        let p=document.getElementById('p'); log.push('upgrade:'+(p.i!==undefined)+':'+(p.i&&p.i.form===document.getElementById('f')));
        log.join(',')
    "#).unwrap();
    assert_eq!(result, JsValue::str("create:true,new:true,upgrade:true:true"));
}

#[test]
fn custom_element_wrappers_keep_expandos_and_prototypes() {
    let mut driver = scheduler("<body><div id=d></div></body>");
    driver.run_script(r#"
        customElements.define('x-b', class extends HTMLElement { constructor(){ super(); this.mark = 7; } });
        document.getElementById('d').expando = 5;
        { let e = document.createElement('x-b'); document.body.appendChild(e); e.id = 'eb'; }
        document.body.insertAdjacentHTML('beforeend', '<x-b id=ib></x-b>');
        0
    "#).unwrap();
    let result = driver.run_script(r#"
        let e = document.getElementById('eb'), i = document.getElementById('ib');
        [document.getElementById('d').expando, e.mark, i.mark, e instanceof customElements.get('x-b'), i instanceof customElements.get('x-b')].join(',')
    "#).unwrap();
    assert_eq!(result, JsValue::str("5,7,7,true,true"));
}

#[test]
fn inner_html_upgrade_runs_form_callbacks_on_pinned_wrappers() {
    let mut driver = scheduler("<body></body>");
    let result = driver.run_script(r#"
        let log=[];
        window.addEventListener('error', e => log.push('err:'+e.message));
        class MyControl extends HTMLElement {
            static get formAssociated() { return true; }
            constructor() { super(); this.internals_ = this.attachInternals(); this.internals_.setFormValue('v'); this.h = []; }
            formDisabledCallback(d) { this.h.push(d); }
            hist() { return this.h; }
        }
        customElements.define('my-control', MyControl);
        const container = document.createElement('div');
        document.body.appendChild(container);
        container.innerHTML = '<my-control disabled>';
        const c = container.firstChild;
        log.push('inst:'+(c instanceof MyControl));
        log.push('hist:'+(typeof c.hist==='function' ? c.hist().join('|') : 'nofn'));
        container.innerHTML = '<fieldset disabled><my-control>';
        const d = container.querySelector('my-control');
        log.push('hist2:'+(typeof d.hist==='function' ? d.hist().join('|') : 'nofn'));
        log.join(',')
    "#).unwrap();
    assert_eq!(result, JsValue::str("inst:true,hist:true,hist2:true"));
}

#[test]
fn inner_html_upgrade_constructs_with_internals() {
    let mut driver = scheduler("<body></body>");
    let result = driver.run_script(r#"
        let log=[];
        window.addEventListener('error', e => log.push('err:'+e.message));
        class A extends HTMLElement { constructor(){ super(); this.a = 1; } }
        class B extends HTMLElement { static get formAssociated() { return true; } constructor(){ super(); this.i = this.attachInternals(); } }
        class C extends HTMLElement { static get formAssociated() { return true; } constructor(){ super(); this.i = this.attachInternals(); this.i.setFormValue('v'); } }
        class D extends HTMLElement { static get formAssociated() { return true; } constructor(){ super(); log.push('D:'+typeof this.attachInternals+':'+(this instanceof HTMLElement)); this.i = this.attachInternals(); } }
        customElements.define('x-a', A); customElements.define('x-b', B); customElements.define('x-c', C); customElements.define('x-d', D);
        const container = document.createElement('div');
        document.body.appendChild(container);
        for (const n of ['a','b','c','d']) {
            container.innerHTML = '<x-'+n+'></x-'+n+'>';
            const e = container.firstChild;
            log.push(n+':'+(n=='a' ? e.a : (e.i === undefined ? 'noi' : 'i')));
        }
        log.join(',')
    "#).unwrap();
    assert_eq!(result, JsValue::str("a:1,b:i,c:i,D:function:true,d:i"));
}

#[test]
fn form_value_validity_labels_states_and_reset_callbacks() {
    let mut driver = scheduler("<body><form id=f><x-face id=c name=n></x-face><label id=l for=c></label></form></body>");
    let result = driver.run_script(r#"
        let log=[];
        class Face extends HTMLElement {
            static get formAssociated(){return true;}
            constructor(){ super(); this.i=this.attachInternals(); }
            formAssociatedCallback(f){ log.push('assoc:'+(f&&f.id)); }
            formResetCallback(){ log.push('reset'); }
            formDisabledCallback(d){ log.push('disabled:'+d); }
        }
        customElements.define('x-face', Face);
        const f=document.getElementById('f'), c=document.getElementById('c');
        log.push('owner:'+(c.i.form===f));
        c.i.setFormValue('v'); log.push('fd:'+new FormData(f).get('n'));
        c.i.setFormValue(new FormData(Object.assign(document.createElement('form'),{})));
        c.i.setValidity({valueMissing:true},'m');
        log.push('valid:'+c.i.validity.valueMissing+':'+c.i.checkValidity()+':'+f.checkValidity()+':'+c.i.validationMessage);
        c.i.setValidity({});
        log.push('labels:'+c.i.labels.length+':'+(c.i.labels[0]===document.getElementById('l')));
        c.i.states.add('--a'); c.i.states.add('--b');
        log.push('states:'+[...c.i.states].join('|')+':'+c.i.states.size);
        c.setAttribute('disabled',''); c.removeAttribute('disabled'); f.reset();
        log.push('fieldset:'+document.createElement('fieldset').elements.length);
        c.remove();
        log.join(',')
    "#).unwrap();
    assert_eq!(result, JsValue::str("assoc:f,owner:true,fd:v,valid:true:false:false:m,labels:1:true,states:--a|--b:2,disabled:true,disabled:false,reset,fieldset:0,assoc:null"));
}
