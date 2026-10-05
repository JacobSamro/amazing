use super::*;

fn interpreter(ast: bool) -> Interpreter {
    let mut value = Interpreter::new();
    value.force_ast_callables = ast;
    value
}

#[test]
fn native_viewport_globals_and_identifier_bindings_follow_logical_sizes_and_density() {
    for ast in [false, true] {
        let mut i = interpreter(ast);
        i.run_script("var readViewport = function(){ return innerWidth + ',' + innerHeight + ',' + devicePixelRatio; }").unwrap();
        let JsValue::Object(reader) = i.run_script("readViewport").unwrap() else { panic!("callable") };
        assert_eq!(matches!(reader.borrow().func.as_ref().unwrap().body, FunctionBody::Bytecode(_)), !ast);
        for (w, h, density) in [(960, 570, 1.), (1280, 800, 2.), (811, 603, 1.25)] {
            i.set_viewport_metrics(w, h, density).unwrap();
            assert_eq!(i.run_script("readViewport()").unwrap(), JsValue::str(format!("{w},{h},{density}")));
            assert_eq!(i.run_script("innerWidth").unwrap(), JsValue::num(w as f64));
            assert_eq!(i.run_script("innerHeight").unwrap(), JsValue::num(h as f64));
            assert_eq!(i.run_script("devicePixelRatio").unwrap(), JsValue::num(density));
            for (name, value) in [("innerWidth", w as f64), ("innerHeight", h as f64),
                ("devicePixelRatio", density)] {
                assert_eq!(i.global_obj.borrow().get_own(name), Some(JsValue::num(value)));
            }
        }
        i.set_viewport_size(320, 240);
        assert_eq!(i.run_script("innerWidth === 320 && innerHeight === 240 && devicePixelRatio === 1").unwrap(), JsValue::Bool(true));
    }
}

#[test]
fn invalid_device_ratio_never_partially_changes_native_metrics_or_bindings() {
    for ast in [false, true] {
        let mut i = interpreter(ast);
        i.set_viewport_metrics(800, 600, 1.5).unwrap();
        for invalid in [0., -0., -1., f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(i.set_viewport_metrics(900, 700, invalid).is_err());
            assert_eq!(i.run_script("innerWidth === 800 && innerHeight === 600 && devicePixelRatio === 1.5").unwrap(), JsValue::Bool(true));
            assert_eq!(i.global_obj.borrow().get_own("innerWidth"), Some(JsValue::num(800.)));
            assert_eq!(i.global_obj.borrow().get_own("innerHeight"), Some(JsValue::num(600.)));
            assert_eq!(i.global_obj.borrow().get_own("devicePixelRatio"), Some(JsValue::num(1.5)));
        }
        i.set_viewport_size(0, 0);
        assert_eq!(i.run_script("innerWidth === 0 && innerHeight === 0").unwrap(), JsValue::Bool(true));
    }
}

#[test]
fn viewport_update_preserves_pending_navigation_history_and_actual_timer_jobs() {
    for ast in [false, true] {
        let mut i = interpreter(ast);
        i.install_location();
        i.install_history();
        i.set_location_url("https://example.test/start");
        i.run_script("history.pushState({marker:1}, '', '/state'); location.href = 'https://example.test/next'; history.go(-1); var viewportTimer = 0; setTimeout(function(){ viewportTimer = innerWidth; }, 0)").unwrap();
        let navigation = i.pending_navigation.clone();
        let history = i.pending_history_delta;
        let update = i.pending_history_update.clone();
        let location = i.location_url.clone();
        let timer_count = i.timers.len();
        let next_timer = i.next_timer_id;
        assert!(timer_count > 0);
        for density in [2., f64::NAN, 1.25] {
            let _ = i.set_viewport_metrics(1200, 700, density);
            assert_eq!(i.pending_navigation, navigation);
            assert_eq!(i.pending_history_delta, history);
            assert_eq!(i.pending_history_update, update);
            assert_eq!(i.location_url, location);
            assert_eq!(i.timers.len(), timer_count);
            assert_eq!(i.next_timer_id, next_timer);
            assert_eq!(i.run_script("viewportTimer").unwrap(), JsValue::num(0.));
        }
        assert!(i.pump_timers().unwrap());
        assert_eq!(i.run_script("viewportTimer").unwrap(), JsValue::num(1200.));
        assert_eq!(i.take_navigation(), navigation);
        assert_eq!(i.take_history_delta(), history);
    }
}
