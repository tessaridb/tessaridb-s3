//! Media renditions of sealed originals (G017 W3c): a rendition is made from the original's plaintext, so it is
//! sealed when the original is; a plain original's rendition stays plain.

use tessari_s3_daemons::{Daemon, ImageWorker, MediaPlanner};

use crate::media::png;
use crate::sse::{SSE, data_files, sse_header, unseal};
use crate::{IGNORED, call, call_with, fresh_with, scratch_dir};

/// The format the header of data file `id` under `dir` declares.
fn format_of(dir: &std::path::Path, id: [u8; 16]) -> u16 {
    let name: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = dir
        .join("s3data")
        .join(&name[0..2])
        .join(&name[2..4])
        .join(&name);
    let bytes = std::fs::read(path).expect("the data file");
    u16::from_le_bytes([bytes[4], bytes[5]])
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_rendition_of_a_sealed_original_is_sealed_and_of_a_plain_one_plain() {
    let _why = IGNORED;
    let (state, planter, dir) = fresh_with(scratch_dir()).await;
    unseal(&state).await;
    assert_eq!(
        call(&state, "PUT", "/pictures", vec![], b"").await.status,
        200
    );
    planter
        .run(
            &format!(
                "UPDATE {}.buckets:'pictures' SET media = [{{ id: 'photos', content_types: ['image/*'], \
                 prefix: 'photos/', outputs: ['image/150/png'] }}];",
                planter.global_database()
            ),
            Vec::new(),
        )
        .await
        .expect("media rules set");
    let image = png(600, 300);
    for (key, sealed) in [("sealed", true), ("plain", false)] {
        let mut headers = vec![("content-type", "image/png")];
        if sealed {
            headers.push(SSE);
        }
        let put = call_with(
            &state,
            "PUT",
            &format!("/pictures/photos/{key}.png"),
            vec![],
            headers,
            &image,
        )
        .await;
        assert_eq!(put.status, 200, "{}", put.body);
    }
    let (media, objects) = (state.storage().media(), state.storage().objects().clone());
    media.define().await.expect("group defined");
    MediaPlanner::new(media.clone(), objects.clone())
        .run_once()
        .await
        .expect("planned");
    ImageWorker::new(media, objects, "node-a".to_owned())
        .run_once()
        .await
        .expect("made");

    for (key, sealed, format) in [("sealed", true, 3), ("plain", false, 1)] {
        let at = format!("photos/{key}.png.renditions/image-150.png");
        let rendition = call(&state, "GET", &format!("/pictures/{at}"), vec![], b"").await;
        assert_eq!(rendition.status, 200, "{}", rendition.body);
        assert_eq!(
            image::load_from_memory(&rendition.bytes)
                .expect("an image")
                .width(),
            150
        );
        assert_eq!(
            sse_header(&rendition).is_some(),
            sealed,
            "{key}: the rendition answers as sealed exactly when its original is"
        );
        let files = data_files(&state, "pictures", &at).await;
        assert_eq!(
            format_of(&dir, files[0]),
            format,
            "{key}: the rendition's data file"
        );
    }
    std::fs::remove_dir_all(dir).expect("cleanup");
}
