use amazing_fetch::api::*;
#[test]
fn blob_file_and_multipart_roundtrip() {
    let blob = Blob::new(
        [
            BlobPart::Text("hello".into()),
            BlobPart::Bytes(vec![0, 255]),
        ],
        "TEXT/PLAIN",
    );
    assert_eq!(blob.content_type, "text/plain");
    assert_eq!(blob.size(), 7);
    assert_eq!(blob.slice(Some(-3), Some(-1), "A/B").bytes, b"o\0");
    assert_eq!(Blob::from_bytes(vec![], "é").content_type, "");
    let file = File::new([BlobPart::Blob(blob)], "a/b.txt", "TEXT/PLAIN", 42);
    assert_eq!(file.name, "a/b.txt");
    assert_eq!(file.last_modified, 42);
    let mut form = FormData::default();
    form.append_text("x\"\r\ny", "a\nb");
    form.append_file("upload", file, Some("new\"name.txt"));
    let (wire, mime) = form.encode_multipart();
    assert!(String::from_utf8_lossy(&wire).contains("name=\"x%22%0D%0Ay\""));
    assert!(String::from_utf8_lossy(&wire).contains("filename=\"new%22name.txt\""));
    let boundary = mime.split("boundary=").nth(1).unwrap();
    let decoded = FormData::decode_multipart(&wire, boundary).unwrap();
    assert_eq!(
        decoded.get("x\"\r\ny"),
        Some(&FormValue::Text("a\r\nb".into()))
    );
    match decoded.get("upload").unwrap() {
        FormValue::File(file) => {
            assert_eq!(file.name, "new\"name.txt");
            assert_eq!(file.blob.bytes, b"hello\0\xff");
            assert_eq!(file.blob.content_type, "text/plain");
        }
        _ => panic!(),
    }
    let mut body = ApiBody::from_input(BodyInput::FormData(form)).unwrap();
    assert!(body
        .mime_type()
        .unwrap()
        .starts_with("multipart/form-data; boundary="));
    assert_eq!(body.form_data().unwrap().entries.len(), 2);
    assert_eq!(body.form_data(), Err(ApiError::BodyUsed));
}
#[test]
fn response_blob_and_urlencoded_form() {
    let mut body = ApiBody::from_input(BodyInput::Blob(Blob::from_bytes(
        b"hi".to_vec(),
        "TEXT/PLAIN",
    )))
    .unwrap();
    assert_eq!(body.mime_type(), Some("text/plain"));
    assert_eq!(body.blob().unwrap().bytes, b"hi");
    let mut body = ApiBody::from_input(BodyInput::UrlSearchParams(vec![(
        "a".encode_utf16().collect(),
        "a b".encode_utf16().collect(),
    )]))
    .unwrap();
    assert_eq!(
        body.form_data().unwrap().get("a"),
        Some(&FormValue::Text("a b".into()))
    );
}
#[test]
fn form_data_rejects_unknown_mime_after_consuming() {
    let mut body = ApiBody::from_input(BodyInput::Bytes(b"a=b".to_vec())).unwrap();
    body.set_mime_type(Some("application/x-www-form-urlencodedgarbage".into()));
    assert_eq!(body.form_data(), Err(ApiError::InvalidFormData));
    assert!(body.body_used());
}

#[test]
fn response_form_data_uses_current_content_type() {
    let mut response = ApiResponse::new(
        Some(BodyInput::Bytes(b"a=b+c".to_vec())),
        ApiResponseInit::default(),
    )
    .unwrap();
    response
        .headers
        .append_string("content-type", "application/x-www-form-urlencoded")
        .unwrap();
    assert_eq!(
        response.form_data().unwrap().get("a"),
        Some(&FormValue::Text("b c".into()))
    );
    assert!(response.body_used());
}
#[test]
fn response_blob_uses_current_content_type() {
    let mut response = ApiResponse::new(
        Some(BodyInput::Bytes(b"x".to_vec())),
        ApiResponseInit::default(),
    )
    .unwrap();
    response
        .headers
        .append_string("content-type", "TEXT/PLAIN")
        .unwrap();
    let blob = response.blob().unwrap();
    assert_eq!(blob.content_type, "text/plain");
    assert_eq!(blob.bytes, b"x");
}

#[test]
fn multipart_rejects_filename_without_field_name() {
    let body =
        b"--b\r\nContent-Disposition: form-data; filename=\"x.txt\"\r\n\r\nhello\r\n--b--\r\n";
    assert_eq!(
        FormData::decode_multipart(body, "b"),
        Err(ApiError::InvalidFormData)
    );
}
#[test]
fn multipart_preserves_semicolon_in_quoted_filename() {
    let mut form = FormData::default();
    form.append_file(
        "f",
        File::new([BlobPart::Bytes(b"x".to_vec())], "a;b.txt", "", 0),
        None,
    );
    let (body, mime) = form.encode_multipart();
    let boundary = mime.split("boundary=").nth(1).unwrap();
    match FormData::decode_multipart(&body, boundary)
        .unwrap()
        .get("f")
        .unwrap()
    {
        FormValue::File(file) => assert_eq!(file.name, "a;b.txt"),
        _ => panic!(),
    }
}
