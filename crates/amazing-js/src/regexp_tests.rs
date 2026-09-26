use crate::value::JsValue;
use crate::{parse_script, Interpreter};
fn truth(source: &str) {
    assert_eq!(
        Interpreter::new().run_script(source).unwrap(),
        JsValue::Bool(true),
        "{source}"
    );
}
#[test]
fn literal_goal_vs_division_and_statement_boundaries() {
    truth(r"var x=12; x/=2; if (x===6) /a/.test('a');");
    truth(r"var x={n:12}; x.n / 2 / 3 === 2 && /[/]/.test('/')");
    truth(r"function f(){return /a+/g;} f().test('aaa') && f() !== f()");
    truth(
        r"var x = /=/.test('='); var y = `prefix${ /[}]/.test('}') }suffix`; x && y === 'prefixtruesuffix'",
    );
    truth(r"var x=8; x / /a/.exec('a').length === 8");
    truth(r"if (true) {} /a/.test('a')");
    truth("function f(){return\n/a/.test('a');} f() === undefined");
}
#[test]
fn regex_literal_errors_are_parse_errors() {
    for source in [
        "/a/gg;",
        "/a/z;",
        "/a\nb/;",
        "/a\\\nb/;",
        "/[/;",
        "/(/;",
        "/[z-a]/;",
        "/a{3,2}/;",
    ] {
        assert!(parse_script(source).is_err(), "{source}");
    }
    assert!(parse_script(r"var r = /\u{1F600}/u;").is_ok());
}
#[test]
fn constructor_source_flags_and_identity() {
    truth(
        r"var r = new RegExp('a+', 'mig'); r.source === 'a+' && r.flags === 'gim' && r.global && r.ignoreCase && r.multiline && !r.sticky && r.lastIndex === 0",
    );
    truth(r"var r=/a/; RegExp(r) === r && new RegExp(r) !== r && new RegExp(r,'i').ignoreCase");
    truth(
        r"new RegExp().source === '(?:)' && /a\/b/.source === 'a\\/b' && /a/i.toString() === '/a/i'",
    );
    truth(r"var error=false;try{new RegExp('[','g')}catch(e){error=e.name==='SyntaxError'} error");
}
#[test]
fn exec_captures_indices_and_state() {
    truth(
        r"var r=/(a)(b)?/g; var m=r.exec('xa'); m[0]==='a' && m[1]==='a' && m[2]===undefined && m.index===1 && m.input==='xa' && m.length===3 && r.lastIndex===2 && r.exec('xa')===null && r.lastIndex===0",
    );
    truth(
        r"var r=/a/y;r.lastIndex=1; r.test('ba') && r.lastIndex===2 && !r.test('ba') && r.lastIndex===0",
    );
    truth(r"var r=/a/;r.lastIndex=20;r.test('a') && r.lastIndex===20");
    truth(r"var r=/(?:)/g;r.exec('x');r.lastIndex===0");
    truth(r"var r=/a/g;r.lastIndex=-1;r.test('a') && r.lastIndex===1");
}
#[test]
fn matching_flags_and_unicode() {
    truth(r"/^[a-z]+$/i.test('AbC') && /^a$/m.test('b\na\nc') && /a.b/s.test('a\nb')");
    truth(r"/\u{1f600}/u.exec('x😀').index===1 && /./gu.exec('😀')[0]==='😀'");
    truth(r"/k/iu.test('K') && !/k/i.test('K') && /\b\w+\b/iu.test('ſ')");
    truth(r"var m=/(z)((a+)?(b+)?(c))*/.exec('zaacbbbcac');m[4]===undefined && m[3]==='a'");
    truth(r"/(?=(a+))a*b\1/.exec('baabac')[0]==='aba'");
}
#[test]
fn string_match_replace_and_search() {
    truth(
        r"'abac'.match(/a/g).length===2 && 'ab'.match(/(a)/)[1]==='a' && 'ab'.match('a')[0]==='a' && 'b'.match(/a/g)===null",
    );
    truth(r"'abc'.match(/(?:)/g).length===4 && '😀'.match(/(?:)/gu).length===2");
    truth(r"'abcabc'.replace(/(a)(b)/g,'$2$1')==='bacbac'");
    truth(r#"'abc'.replace(/(b)/,"[$$][$&][$`][$']")==='a[$][b][a][c]c'"#);
    truth(r"'abc'.replace(/(b)/,'$01-$10-$99')==='ab-b0-$99c'");
    truth(
        r"'a1a2'.replace(/a(\d)/g,function(full,digit,index,source){return digit+index;})==='1022'",
    );
    truth(r"var r=/b/g;r.lastIndex=2;'abc'.search(r)===1 && r.lastIndex===2");
    truth(r"'pass();  '.replace(/^([^;]*)(;\s*)+$/,'$1')==='pass()'");
}
#[test]
fn readonly_last_index_and_overridden_exec() {
    truth(
        r"var r=/a/g;Object.defineProperty(r,'lastIndex',{writable:false});var error=false;try{r.exec('a')}catch(e){error=e.name==='TypeError'}error",
    );
    truth(r"var r=/a/;r.exec=function(){return null;};r.test('a')===false");
    truth(
        r"var r=/a/;r.exec=function(){return 3;};var error=false;try{r.test('a')}catch(e){error=e.name==='TypeError'}error",
    );
}
#[test]
fn bounded_pathological_pattern_throws() {
    truth(
        "var r=/(a+)+$/;var s='aaaaaaaaaaaaaaaaaaaaaaaaaaaa!';var limit=false;try{r.test(s)}catch(e){limit=e.name==='RangeError'}limit",
    );
}

#[test]
fn named_capture_groups() {
    truth(
        r"var m=/(?<year>\d{4})-(?<month>\d{2})/.exec('2024-06');m.groups.year==='2024' && m.groups.month==='06' && m[1]==='2024'",
    );
    truth(r"var m=/(?<a>x)?/.exec('y');m.groups.a===undefined && Object.getPrototypeOf(m.groups)===null");
    truth(r"/(?<a>x)/.exec('y')===null");
    truth(r"/(?<a>x)\k<a>/.test('xx') && !/(?<a>x)\k<a>/.test('xy')");
    truth(r"/\k<a>(?<a>x)/.test('x')");
    truth(r"'2024-06'.replace(/(?<year>\d{4})-(?<month>\d{2})/,'$<month>/$<year>')==='06/2024'");
    truth(
        r"'2024-06'.replace(/(?<year>\d{4})-(?<month>\d{2})/,function(full,year,month,index,source,groups){return groups.year+'!'+groups.month;})==='2024!06'",
    );
    truth(r"var error=false;try{new RegExp('(?<a>x)(?<a>y)')}catch(e){error=e.name==='SyntaxError'}error");
    // Annex B: with no named group anywhere in the pattern, `\k<a>` is a
    // plain identity escape (literal `k`), not a forced backreference.
    truth(r"new RegExp('\\k<a>').test('k<a>')");
    truth(r"var error=false;try{new RegExp('(?<b>x)\\k<a>')}catch(e){error=e.name==='SyntaxError'}error");
}
#[test]
fn regexp_escape_static_method() {
    truth(r"RegExp.escape('foo bar')==='\\x66oo\\x20bar'");
    truth(r"RegExp.escape('a+b*c')==='\\x61\\+b\\*c'");
    truth(r"RegExp.escape('1a')==='\\x31a' && RegExp.escape('a1')==='\\x611'");
    truth(r"new RegExp('^'+RegExp.escape('a.b')+'$').test('a.b') && !new RegExp('^'+RegExp.escape('a.b')+'$').test('axb')");
    truth(r"RegExp.escape.length===1 && RegExp.escape.name==='escape'");
    truth(r"var error=false;try{RegExp.escape(1)}catch(e){error=e.name==='TypeError'}error");
}
#[test]
fn symbol_match_search_and_match_all() {
    truth(r"/a/[Symbol.match]('cat')[0]==='a'");
    truth(r"/a/g[Symbol.match]('banana').join(',')==='a,a,a'");
    truth(r"/z/g[Symbol.match]('banana')===null");
    truth(r"/an/[Symbol.search]('banana')===1");
    truth(r"/z/[Symbol.search]('banana')===-1");
    truth(
        r"var r=/a/g;r.lastIndex=5;r[Symbol.search]('banana');r.lastIndex===5",
    );
    truth(
        r"var it=/a/g[Symbol.matchAll]('banana');var out=[];var step;while(!(step=it.next()).done){out.push(step.value[0]+':'+step.value.index);}out.join(',')==='a:1,a:3,a:5'",
    );
    truth(r"var it=/a/g[Symbol.matchAll]('banana');it[Symbol.iterator]()===it");
    truth(
        r"var it=/(?<x>a)/g[Symbol.matchAll]('banana');it.next().value.groups.x==='a'",
    );
    truth(
        r"var it=/a/[Symbol.matchAll]('banana');var first=it.next();var second=it.next();first.done===false && first.value[0]==='a' && second.done===true && second.value===undefined",
    );
    truth(r"var r=/a/g;r.lastIndex=3;var it=r[Symbol.matchAll]('banana');r.lastIndex===3 && it.next().value.index===3");
}
#[test]
fn templates_in_blocks_and_getter_receivers() {
    truth(r"function f(){return `a${ /[}]/.test('}') }${ `${({a:2}).a}` }`;} f()==='atrue2'");
    truth(
        r"var r=/a/;Object.defineProperty(r,'global',{get:function(){r.x=1;return true;}});r.flags==='g' && r.x===1",
    );
}

#[test]
fn harness_control_character_pattern() {
    truth(
        r"String.fromCharCode(65, 65536 + 66, 0xd83d, 0xde00)==='AB😀' && String.fromCharCode().length===0 && String.fromCharCode(NaN, Infinity).length===2",
    );
    truth(
        r#"function f(){var map={};for(var p in {'10':'n','13':'r'}){map[String.fromCharCode(p)]='x';}var r=new RegExp(`[${Object.keys(map).map(k => k).join('')}]`, 'g');return 'a\nb\r'.replace(r,'x');} f()==='axbx'"#,
    );
}

#[test]
fn harness_exports_are_global_bindings() {
    truth("globalThis.js2_export = function(){return 7;}; typeof js2_export==='function' && js2_export()===7");
    truth("globalThis.js2_counter=2;js2_counter+=3;globalThis.js2_counter===5");
    truth("globalThis.js2_shadow=2;function f(){let js2_shadow=3;return js2_shadow;}f()===3 && js2_shadow===2");
    truth("Object.defineProperty(globalThis,'js2_get',{get:function(){return 4;}});js2_get===4 && typeof js2_get==='number'");
}

#[test]
fn harness_setup_uses_actual_argument_count_and_function_identity() {
    truth("function f(a,b){return arguments.length;}f(1)===1 && f(1,2,3)===3 && f()===0");
    truth("function f(a,...rest){return rest.join(',');} f(1,2,3)==='2,3' && f(1)===''");
    truth("function f(){};f instanceof Function && !({} instanceof Function) && Function('a','return a+1;')(2)===3");
    truth("var p={a:1};p.hasOwnProperty('a') && !p.hasOwnProperty('toString')");
    truth("function f(){return Array.prototype.slice.call(arguments,1).join(',');} f(1,2,3)==='2,3' && [] instanceof Array");
}

#[test]
fn harness_completion_some_skips_holes_and_short_circuits() {
    truth("var calls=0;var a=[,2,3];a.some(function(v,k,o){calls++;return v===2 && k===1 && o===a;}) && calls===1 && ![].some(function(){return true;})");
}

#[test]
fn lookbehind_assertions_end_to_end() {
    truth(r"/(?<=\$)\d+/.exec('$100')[0]==='100'");
    truth(r"/(?<=\$)\d+/.test('100')===false");
    truth(r"/(?<!\$)\b\d+\b/.test('200')===true && /(?<!\$)\d+/.exec('$100')[0]==='00'");
    truth(r"/(?<=(\d+)-)\w+/.exec('42-foo')[1]==='42'");
    truth(r"var error=false;try{new RegExp('(?<=a)+')}catch(e){error=e.name==='SyntaxError'}error");
}

#[test]
fn inline_modifier_groups_end_to_end() {
    truth(r"/(?i:a)b/.test('Ab') && !/(?i:a)b/.test('AB')");
    truth(r"/(?-i:a)b/i.test('aB') && !/(?-i:a)b/i.test('Ab')");
    truth(r"/(?s:a.)b/.test('a\nb') && !/a.b/.test('a\nb')");
    truth(r"/(?m:^b)/.test('a\nb') && !/^b/.test('a\nb')");
}

#[test]
fn has_indices_flag_and_indices_property() {
    truth(r"/a/d.hasIndices===true && /a/.hasIndices===false");
    truth(r"/a/d.flags==='d'");
    truth(r"var m=/(?<x>b)c/d.exec('abc');m.indices[0][0]===1 && m.indices[0][1]===3");
    truth(r"var m=/(?<x>b)c/d.exec('abc');m.indices[1][0]===1 && m.indices[1][1]===2");
    truth(r"var m=/(?<x>b)?c/d.exec('c');m.indices[1]===undefined");
    truth(r"var m=/(?<x>b)c/d.exec('abc');m.indices.groups.x[0]===1 && m.indices.groups.x[1]===2");
    truth(r"/a/.exec('a').indices===undefined");
}

#[test]
fn symbol_split() {
    truth(r"/,/[Symbol.split]('a,b,,c').join('|')==='a|b||c'");
    truth(r"/,/[Symbol.split]('a,b,c',2).join('|')==='a|b'");
    truth(r"/(,)/[Symbol.split]('a,b').join('|')==='a|,|b'");
    truth(r"/(?:)/[Symbol.split]('abc').join('|')==='a|b|c'");
    truth(r"/,/[Symbol.split]('').length===1 && /,/[Symbol.split]('')[0]===''");
    truth(r"var r=/a*/[Symbol.split]('ab');r.length===2 && r[0]==='' && r[1]==='b'");
}
