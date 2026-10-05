use super::*;

#[test]
fn fallible_merge_sort_is_stable_and_bounded_for_large_reversed_inputs() {
    let mut values: Vec<_> = (0..16384).rev().map(|n| (n % 31, n)).collect();
    let mut calls = 0;
    binary_stable_merge_sort(&mut values, &mut |a, b| {
        calls += 1;
        Ok::<_, ()>(a.0.cmp(&b.0))
    }).unwrap();
    assert!(calls < 16384 * 14);
    assert!(values.windows(2).all(|w| w[0].0 <= w[1].0));
    // The captured order was descending; ties must retain that exact order.
    assert!(values.windows(2).filter(|w| w[0].0 == w[1].0).all(|w| w[0].1 > w[1].1));
}

#[test]
fn fallible_merge_sort_propagates_original_exception_and_accepts_inconsistent_order() {
    let mut values: Vec<_> = (0..1024).rev().collect();
    let mut calls = 0;
    let result = binary_stable_merge_sort(&mut values, &mut |a, b| {
        calls += 1;
        if calls == 7 { Err(123) } else { Ok(a.cmp(b)) }
    });
    assert_eq!(result, Err(123));
    assert_eq!(calls, 7);
    let mut values: Vec<_> = (0..1024).collect();
    binary_stable_merge_sort(&mut values, &mut |_, _| Ok::<_, ()>(std::cmp::Ordering::Greater)).unwrap();
    values.sort();
    assert_eq!(values, (0..1024).collect::<Vec<_>>());
}

#[test]
fn typedarray_sort_whole_constructor_callback_and_snapshot_matrix() {
    let source = r#"
      function check(ok,label){if(!ok)throw label;}
      var constructors=[Int8Array,Uint8Array,Uint8ClampedArray,Int16Array,Uint16Array,Int32Array,Uint32Array,Float16Array,Float32Array,Float64Array,BigInt64Array,BigUint64Array];
      for(var C of constructors){
        var big=C===BigInt64Array||C===BigUint64Array;
        var input=big?[4n,1n,3n,1n]:[4,1,3,1];
        for(var method of ['sort','toSorted']){
          var a=new C(input),calls=0;
          var result=a[method](function(x,y){calls++;check(this===undefined,'strict receiver');return x<y?-1:x>y?1:0;});
          check(result.join(',')==='1,1,3,4'&&calls>0,C.name+'/'+method);
          check((method==='sort')===(result===a),'result identity');
          if(method==='toSorted')check(a.join(',')==='4,1,3,1','original retained');
          var sentinel={},calls=0,b=new C(input),caught;
          try{b[method](function(){calls++;throw sentinel;});}catch(e){caught=e;}
          check(caught===sentinel&&calls===1&&b.join(',')==='4,1,3,1','throw before writes');
          var c=new C(input);
          var same=c[method](function(){return NaN;});
          check(same.join(',')==='4,1,3,1','NaN comparison is stable equality');
          var d=new C(input),changed=false;
          var captured=d[method](function(x,y){if(!changed){changed=true;d.fill(big?9n:9);}return x<y?-1:x>y?1:0;});
          check(captured.join(',')==='1,1,3,4','captured list survives callback mutation');
          if(method==='toSorted')check(d.join(',')==='9,9,9,9','copy-sort preserves callback effects');
        }
      }
      for(var C of [Float16Array,Float32Array,Float64Array]){
        var a=new C([NaN,0,-0,Infinity,-Infinity,3,NaN]);
        a.sort();
        check(a[0]===-Infinity&&Object.is(a[1],-0)&&Object.is(a[2],0)&&a[3]===3&&a[4]===Infinity&&Number.isNaN(a[5])&&Number.isNaN(a[6]),'numeric signed-zero NaN order');
      }
      true;
    "#;
    // Make callback strict without altering the enclosing suite's semantics.
    let source = source.replace("function(x,y){calls++;", "function(x,y){'use strict';calls++;");
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut interpreter = Interpreter::new();
        let actual = interpreter.run_script_with_backend(&source, backend);
        assert!(matches!(actual, Ok(JsValue::Bool(true))), "{backend:?}: {actual:?}");
    }
}
