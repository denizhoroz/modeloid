//! Annotation formats: the one list every format picker and format name
//! comes from (Download's Roboflow export, Create, Convert, Classes &
//! Merge, the workspace's default).
//!
//! A format's `id` is its Roboflow export name when `roboflow` is set,
//! otherwise a name of our own. `writes_as` is the label family Modeloid
//! reads and writes it as ("yolo", "coco", "voc"); `None` means Modeloid
//! can't read or write it yet (it can still be downloaded from Roboflow).

/// Label families Modeloid reads and writes.
pub const YOLO: &str = "yolo";
pub const COCO: &str = "coco";
pub const VOC: &str = "voc";

/// What a format's labels describe; pickers group formats by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatTask {
    /// Boxes, and outlines where the format has them.
    Detection,
    OrientedBoxes,
    Keypoints,
    SemanticSegmentation,
    Classification,
    Multimodal,
}

impl FormatTask {
    pub const ALL: [FormatTask; 6] = [
        FormatTask::Detection,
        FormatTask::OrientedBoxes,
        FormatTask::Keypoints,
        FormatTask::SemanticSegmentation,
        FormatTask::Classification,
        FormatTask::Multimodal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FormatTask::Detection => "Object detection / segmentation",
            FormatTask::OrientedBoxes => "Oriented boxes (OBB)",
            FormatTask::Keypoints => "Keypoints / pose",
            FormatTask::SemanticSegmentation => "Semantic segmentation",
            FormatTask::Classification => "Classification",
            FormatTask::Multimodal => "Multimodal / vision-language",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnotationFormat {
    pub id: &'static str,
    pub label: &'static str,
    pub task: FormatTask,
    /// Label family Modeloid reads and writes it as; `None` = not supported yet.
    pub writes_as: Option<&'static str>,
    /// Roboflow can export it (`id` is the export name).
    pub roboflow: bool,
}

impl AnnotationFormat {
    pub fn supported(&self) -> bool {
        self.writes_as.is_some()
    }
}

const fn f(id: &'static str, label: &'static str, task: FormatTask, writes_as: Option<&'static str>, roboflow: bool) -> AnnotationFormat {
    AnnotationFormat { id, label, task, writes_as, roboflow }
}

use FormatTask::*;

/// Every format, grouped by task, the usual choice first in each family.
/// The YOLO PyTorch TXT versions (YOLO26 back to YOLOv5) share one layout:
/// a txt per image, images/ and labels/ per split, data.yaml.
pub const FORMATS: &[AnnotationFormat] = &[
    // Object detection / segmentation
    f("yolo26", "YOLO26 TXT", Detection, Some(YOLO), true),
    f("yolov12", "YOLOv12 TXT", Detection, Some(YOLO), true),
    f("yolov11", "YOLO11 TXT", Detection, Some(YOLO), true),
    f("yolov10", "YOLOv10 TXT", Detection, Some(YOLO), true),
    f("yolov9", "YOLOv9 TXT", Detection, Some(YOLO), true),
    f("yolov8", "YOLOv8 TXT", Detection, Some(YOLO), true),
    f("yolov7pytorch", "YOLOv7 PyTorch TXT", Detection, Some(YOLO), true),
    f("mt-yolov6", "YOLOv6 (Meituan) TXT", Detection, Some(YOLO), true),
    f("yolov5pytorch", "YOLOv5 PyTorch TXT", Detection, Some(YOLO), true),
    f("coco", "COCO JSON", Detection, Some(COCO), true),
    f("coco-segmentation", "COCO Segmentation JSON", Detection, Some(COCO), true),
    f("voc", "Pascal VOC XML", Detection, Some(VOC), true),
    f("yolov4scaled", "Scaled-YOLOv4 TXT", Detection, None, true),
    f("yolov4pytorch", "YOLOv4 PyTorch TXT", Detection, None, true),
    f("darknet", "YOLO Darknet TXT", Detection, None, true),
    f("yolokeras", "YOLO Keras TXT", Detection, None, true),
    f("createml", "CreateML JSON", Detection, None, true),
    f("tensorflow", "TensorFlow Object Detection CSV", Detection, None, true),
    f("tfrecord", "TensorFlow TFRecord", Detection, None, true),
    f("retinanet", "RetinaNet Keras CSV", Detection, None, true),
    f("labelme", "LabelMe JSON", Detection, None, false),
    f("cvat", "CVAT for images XML", Detection, None, false),
    f("label-studio", "Label Studio JSON", Detection, None, false),
    f("kitti", "KITTI TXT", Detection, None, false),
    f("openimages", "Open Images CSV", Detection, None, false),
    f("sagemaker", "SageMaker Ground Truth manifest", Detection, None, false),
    f("automl", "Google Cloud AutoML / Vertex AI CSV", Detection, None, false),
    f("supervisely", "Supervisely JSON", Detection, None, false),
    // Oriented boxes
    f("yolov8-obb", "YOLOv8 OBB TXT", OrientedBoxes, None, true),
    f("yolov5-obb", "YOLOv5 OBB TXT", OrientedBoxes, None, true),
    f("dota", "DOTA TXT", OrientedBoxes, None, false),
    // Keypoints
    f("yolo-pose", "YOLO Pose TXT", Keypoints, None, false),
    f("coco-keypoints", "COCO Keypoints JSON", Keypoints, None, false),
    // Semantic segmentation
    f("png-mask-semantic", "Semantic masks (PNG)", SemanticSegmentation, None, true),
    f("cityscapes", "Cityscapes", SemanticSegmentation, None, false),
    // Classification
    f("folder", "Folder structure", Classification, None, true),
    f("multiclass", "Multi-label CSV", Classification, None, true),
    f("clip", "OpenAI CLIP", Classification, None, true),
    // Multimodal
    f("paligemma", "PaliGemma JSONL", Multimodal, None, true),
    f("florence2-od", "Florence-2 JSONL", Multimodal, None, true),
];

/// Download's starting format.
pub const DEFAULT_DOWNLOAD_FORMAT: &str = "coco";

/// The format called `id` (any case).
pub fn find(id: &str) -> Option<&'static AnnotationFormat> {
    let id = id.trim();
    FORMATS.iter().find(|f| f.id.eq_ignore_ascii_case(id))
}

/// The label family (`YOLO`, `COCO`, `VOC`) Modeloid reads `name` as: a
/// format id, a family name, or an unknown name that looks like a YOLO /
/// COCO / VOC one (e.g. a newer "yolov13"). OBB, pose and Keras YOLO
/// variants are different layouts and give `None`.
pub fn family(name: &str) -> Option<&'static str> {
    let name = name.trim().to_ascii_lowercase();
    match name.as_str() {
        YOLO => return Some(YOLO),
        COCO => return Some(COCO),
        VOC | "pascal-voc" => return Some(VOC),
        _ => {}
    }
    if let Some(format) = find(&name) {
        return format.writes_as;
    }
    let other_layout = ["obb", "pose", "keypoint", "keras", "darknet"].iter().any(|w| name.contains(w));
    if name.starts_with(YOLO) && !other_layout {
        Some(YOLO)
    } else if name.starts_with(COCO) && !other_layout {
        Some(COCO)
    } else if name.starts_with(VOC) {
        Some(VOC)
    } else {
        None
    }
}

/// The format a stored name shows as: itself, or, for a family name
/// ("yolo"), the family's first format.
pub fn resolve(name: &str) -> Option<&'static AnnotationFormat> {
    find(name).or_else(|| {
        let family = family(name)?;
        FORMATS.iter().find(|f| f.writes_as == Some(family))
    })
}

/// A name to show for `name`: the format's label, or the name itself.
pub fn label(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        YOLO => "YOLO".into(),
        COCO => "COCO".into(),
        VOC => "Pascal VOC".into(),
        _ => find(name).map_or_else(|| name.to_string(), |f| f.label.to_string()),
    }
}

/// Formats Modeloid can write, e.g. for a new dataset.
pub fn writable(name: &str) -> bool {
    family(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        for (i, a) in FORMATS.iter().enumerate() {
            assert!(FORMATS[i + 1..].iter().all(|b| b.id != a.id), "{} twice", a.id);
            assert!(a.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'), "{} isn't a valid export name", a.id);
        }
        assert!(find(DEFAULT_DOWNLOAD_FORMAT).is_some_and(|f| f.roboflow));
    }

    #[test]
    fn families() {
        assert_eq!(family("yolo26"), Some(YOLO));
        assert_eq!(family("YOLOv8"), Some(YOLO));
        assert_eq!(family("mt-yolov6"), Some(YOLO));
        assert_eq!(family("yolov13"), Some(YOLO), "a newer YOLO version");
        assert_eq!(family("coco-segmentation"), Some(COCO));
        assert_eq!(family("pascal-voc"), Some(VOC));
        assert_eq!(family("yolov8-obb"), None);
        assert_eq!(family("yolokeras"), None);
        assert_eq!(family("createml"), None);
        assert_eq!(family(""), None);
    }

    #[test]
    fn resolves_family_names() {
        assert_eq!(resolve("yolo").map(|f| f.id), Some("yolo26"));
        assert_eq!(resolve("voc").map(|f| f.id), Some("voc"));
        assert_eq!(resolve("yolov8").map(|f| f.id), Some("yolov8"));
        assert_eq!(resolve("parquet"), None);
        assert_eq!(label("yolo"), "YOLO");
        assert_eq!(label("yolov11"), "YOLO11 TXT");
        assert_eq!(label("parquet"), "parquet");
    }
}
