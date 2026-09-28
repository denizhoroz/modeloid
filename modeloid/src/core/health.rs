//! Health Check: finds broken images, bad boxes, unknown classes, label files
//! without images, and duplicate images (identical files, and look-alikes via
//! a perceptual hash), including duplicates leaking across splits. Fixes are
//! applied to a `LabelSet`, which the caller writes out as a new dataset.

use std::{
    collections::{HashMap, HashSet},
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::core::{
    io::run_parallel,
    labels::{BBox, LabelSet},
    progress::Progress,
    stats::split_slot,
};

pub use modeloid_types::health::*;

/// Look-alike threshold: perceptual hashes differing in at most this many of
/// 64 bits. With 4 bands of 16 bits, two such hashes share a band exactly
/// (pigeonhole), so only images sharing a band are compared.
const NEAR_DISTANCE: u32 = 3;
const BANDS: usize = 4;
/// Boxes may overshoot the image edge by this much (rounding) before counting.
const EDGE_TOLERANCE: f64 = 0.5;

/// What reading one image file found.
#[derive(Default)]
struct Scan {
    len: u64,
    hash: u64,
    dhash: Option<u64>,
    error: Option<String>,
    size: Option<(u32, u32)>,
}

/// Checks every image of `set` (read from `dir`) and its labels. Decoding
/// every image is the slow part; it runs on all cores and counts on `progress`.
pub fn check(
    dir: &Path,
    set: &LabelSet,
    orphans: Vec<PathBuf>,
    stamp: String,
    progress: &dyn Progress,
) -> Result<HealthReport, crate::Error> {
    progress.set_total(set.images.len() as u64);
    let scans: Mutex<Vec<Scan>> = Mutex::new((0..set.images.len()).map(|_| Scan::default()).collect());
    let indexes: Vec<usize> = (0..set.images.len()).collect();
    run_parallel(&indexes, progress, || Ok(()), |_, &i| {
        let scan = scan_image(&set.images[i].file);
        scans.lock().unwrap()[i] = scan;
        Ok(())
    })?;
    let scans = scans.into_inner().unwrap();

    let image_ref = |i: usize| ImageRef { index: i, path: rel_path(dir, &set.images[i].file), split: set.images[i].split };
    let mut report = HealthReport {
        version: HEALTH_VERSION,
        stamp,
        checked_at: chrono::Utc::now().to_rfc3339(),
        image_count: set.images.len() as u32,
        label_count: set.images.iter().map(|i| i.objects.len() as u32).sum(),
        classes: set.classes.clone(),
        corrupt_images: Vec::new(),
        size_mismatches: Vec::new(),
        unlabeled_images: Vec::new(),
        orphan_labels: orphans.iter().map(|p| slashes(p)).collect(),
        label_warnings: set.warnings.clone(),
        degenerate_boxes: Vec::new(),
        out_of_bounds: Vec::new(),
        unknown_classes: Vec::new(),
        duplicates: Vec::new(),
    };

    for (i, (image, scan)) in set.images.iter().zip(&scans).enumerate() {
        if let Some(error) = &scan.error {
            report.corrupt_images.push(ImageIssue { image: image_ref(i), detail: error.clone() });
        } else if let Some((w, h)) = scan.size.filter(|&s| s != (image.width, image.height)) {
            let detail = format!("labels assume {}×{}, the file is {w}×{h}", image.width, image.height);
            report.size_mismatches.push(ImageIssue { image: image_ref(i), detail });
        }
        // A corrupt image is reported once, as corrupt.
        if image.objects.is_empty() && scan.error.is_none() {
            report.unlabeled_images.push(image_ref(i));
        }
        for (n, object) in image.objects.iter().enumerate() {
            let issue = |detail: String| BoxIssue { image: image_ref(i), object: n, class: object.class, detail };
            if let Some(problem) = degenerate(&object.bbox) {
                report.degenerate_boxes.push(issue(problem.to_string()));
            } else if let Some(over) = overshoot(&object.bbox, image.width, image.height) {
                report.out_of_bounds.push(issue(format!("reaches {over:.0} px outside the image")));
            }
            if set.declared_classes.is_some_and(|declared| object.class >= declared) {
                report.unknown_classes.push(issue(format!("class id {} isn't in the class list", object.class)));
            }
        }
    }

    report.duplicates = duplicate_groups(&scans, |i| image_ref(i));
    Ok(report)
}

fn scan_image(path: &Path) -> Scan {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => return Scan { error: Some(format!("can't read the file: {e}")), ..Default::default() },
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let mut scan = Scan { len: bytes.len() as u64, hash: hasher.finish(), ..Default::default() };
    match image::load_from_memory(&bytes) {
        Ok(img) => {
            scan.size = Some((img.width(), img.height()));
            scan.dhash = Some(dhash(&img));
        }
        Err(e) => scan.error = Some(format!("can't decode: {e}")),
    }
    scan
}

/// Difference hash: 9×8 grayscale thumbnail, one bit per "left pixel brighter
/// than its right neighbour". Similar images differ in few bits.
fn dhash(img: &image::DynamicImage) -> u64 {
    let small = img.resize_exact(9, 8, image::imageops::FilterType::Triangle).to_luma8();
    let mut bits = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            bits = (bits << 1) | u64::from(small.get_pixel(x, y)[0] > small.get_pixel(x + 1, y)[0]);
        }
    }
    bits
}

/// Why a box can't be right, if it can't.
pub fn degenerate(b: &BBox) -> Option<&'static str> {
    if !(b.w > 0.0 && b.h > 0.0) {
        Some("no width or height")
    } else if b.area() < 1.0 {
        Some("smaller than one pixel")
    } else {
        None
    }
}

/// How far (px) a box reaches past the image edges, beyond rounding.
pub fn overshoot(b: &BBox, width: u32, height: u32) -> Option<f64> {
    let (w, h) = (f64::from(width), f64::from(height));
    let over = (-b.x).max(-b.y).max(b.x + b.w - w).max(b.y + b.h - h);
    (over > EDGE_TOLERANCE).then_some(over)
}

/// Identical files (same size and content hash), then look-alikes among the rest.
fn duplicate_groups(scans: &[Scan], image_ref: impl Fn(usize) -> ImageRef) -> Vec<DuplicateGroup> {
    let mut groups = Vec::new();

    let mut exact: HashMap<(u64, u64), Vec<usize>> = HashMap::new();
    for (i, scan) in scans.iter().enumerate().filter(|(_, s)| s.error.is_none()) {
        exact.entry((scan.len, scan.hash)).or_default().push(i);
    }
    // One image per identical group takes part in the look-alike search.
    let mut representatives = Vec::new();
    for members in exact.values() {
        representatives.push(members[0]);
        if members.len() > 1 {
            groups.push((true, members.clone()));
        }
    }
    representatives.sort_unstable();

    let mut parent: Vec<usize> = (0..scans.len()).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut root = i;
        while parent[root] != root {
            root = parent[root];
        }
        parent[i] = root;
        root
    }
    for band in 0..BANDS {
        let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
        for &i in &representatives {
            if let Some(h) = scans[i].dhash {
                buckets.entry((h >> (band * 16)) & 0xFFFF).or_default().push(i);
            }
        }
        for bucket in buckets.values().filter(|b| b.len() > 1) {
            for (a_pos, &a) in bucket.iter().enumerate() {
                for &b in &bucket[a_pos + 1..] {
                    if (scans[a].dhash.unwrap() ^ scans[b].dhash.unwrap()).count_ones() <= NEAR_DISTANCE {
                        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                        parent[ra] = rb;
                    }
                }
            }
        }
    }
    let mut near: HashMap<usize, Vec<usize>> = HashMap::new();
    for &i in &representatives {
        let root = find(&mut parent, i);
        near.entry(root).or_default().push(i);
    }
    groups.extend(near.into_values().filter(|m| m.len() > 1).map(|m| (false, m)));

    let mut groups: Vec<DuplicateGroup> = groups
        .into_iter()
        .map(|(exact, members)| {
            let mut images: Vec<ImageRef> = members.into_iter().map(&image_ref).collect();
            images.sort_by(|a, b| (split_slot(a.split), &a.path).cmp(&(split_slot(b.split), &b.path)));
            DuplicateGroup { exact, images }
        })
        .collect();
    groups.sort_by(|a, b| a.images[0].path.cmp(&b.images[0].path));
    groups
}

/// What `apply_fixes` changed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FixSummary {
    pub removed_images: usize,
    pub dropped_objects: usize,
    pub clipped_objects: usize,
}

/// Applies `fixes` to `set` (read from `dir`), using `report` for the image
/// findings (decoding, duplicates). Box fixes re-apply the same rules.
pub fn apply_fixes(dir: &Path, set: &mut LabelSet, report: &HealthReport, fixes: &HealthFixes) -> FixSummary {
    let mut summary = FixSummary::default();
    let mut remove: HashSet<&str> = HashSet::new();
    if fixes.remove_corrupt {
        remove.extend(report.corrupt_images.iter().map(|i| i.image.path.as_str()));
    }
    for group in &report.duplicates {
        let keep = &group.images[0];
        let all = (group.exact && fixes.remove_exact_duplicates) || (!group.exact && fixes.remove_near_duplicates);
        for image in &group.images[1..] {
            if all || (fixes.remove_leaks && image.split != keep.split) {
                remove.insert(image.path.as_str());
            }
        }
    }

    let before = set.images.len();
    set.images.retain(|image| !remove.contains(rel_path(dir, &image.file).as_str()));
    summary.removed_images += before - set.images.len();

    let declared = set.declared_classes;
    for image in &mut set.images {
        let (w, h) = (image.width, image.height);
        let before = image.objects.len();
        image.objects.retain(|o| {
            !(fixes.drop_degenerate && degenerate(&o.bbox).is_some())
                && !(fixes.drop_unknown_classes && declared.is_some_and(|d| o.class >= d))
        });
        summary.dropped_objects += before - image.objects.len();

        if fixes.clip_out_of_bounds {
            for object in &mut image.objects {
                if degenerate(&object.bbox).is_none() && overshoot(&object.bbox, w, h).is_some() {
                    object.bbox = clip(object.bbox, w, h);
                    if let Some(points) = &mut object.polygon {
                        for [x, y] in points.iter_mut() {
                            *x = x.clamp(0.0, f64::from(w));
                            *y = y.clamp(0.0, f64::from(h));
                        }
                    }
                    summary.clipped_objects += 1;
                }
            }
            // A box entirely outside the image clips to nothing.
            let before = image.objects.len();
            image.objects.retain(|o| degenerate(&o.bbox).is_none());
            summary.dropped_objects += before - image.objects.len();
        }
    }
    if fixes.drop_unknown_classes {
        if let Some(declared) = declared {
            set.classes.truncate(declared);
        }
    }
    if fixes.remove_unlabeled {
        let before = set.images.len();
        set.images.retain(|image| !image.objects.is_empty());
        summary.removed_images += before - set.images.len();
    }
    summary
}

fn clip(b: BBox, width: u32, height: u32) -> BBox {
    let (w, h) = (f64::from(width), f64::from(height));
    let (x0, y0) = (b.x.clamp(0.0, w), b.y.clamp(0.0, h));
    let (x1, y1) = ((b.x + b.w).clamp(0.0, w), (b.y + b.h).clamp(0.0, h));
    BBox { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

/// `file` relative to `dir`, with `/` separators on every system.
pub fn rel_path(dir: &Path, file: &Path) -> String {
    slashes(file.strip_prefix(dir).unwrap_or(file))
}

fn slashes(path: &Path) -> String {
    path.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::labels::{self, LabelFormat, Split};
    use crate::core::progress::Silent;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("modeloid-health-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn write(&self, rel: &str, contents: impl AsRef<[u8]>) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }

        fn png(&self, rel: &str, seed: u8) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            // A gradient whose direction depends on the seed, so different seeds look different.
            // Brightness rises left to right for even seeds and falls for odd ones.
            let img = image::RgbImage::from_fn(64, 48, |x, _| {
                let v = (if seed % 2 == 0 { x * 4 } else { 255 - x * 4 }) as u8;
                image::Rgb([v, seed * 10, v])
            });
            img.save(path).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A small YOLO dataset with one of each problem.
    fn broken_dataset(name: &str) -> TempDir {
        let tmp = TempDir::new(name);
        tmp.write("data.yaml", "names: ['cat', 'dog']\n");
        tmp.png("train/images/a.png", 0);
        tmp.write("train/labels/a.txt", "0 0.5 0.5 0.2 0.2\n1 0.9 0.5 0.4 0.2\n0 0.5 0.5 0 0.1\n5 0.5 0.5 0.1 0.1\n");
        tmp.png("train/images/b.png", 1);
        fs::create_dir_all(tmp.0.join("test/images")).unwrap();
        fs::copy(tmp.0.join("train/images/b.png"), tmp.0.join("test/images/b-copy.png")).unwrap(); // leaks into test
        tmp.write("test/labels/b-copy.txt", "0 0.5 0.5 0.2 0.2\n");
        // Valid PNG header (so the reader keeps it) but no pixel data.
        let a = fs::read(tmp.0.join("train/images/a.png")).unwrap();
        tmp.write("valid/images/broken.png", &a[..40]);
        tmp.write("valid/labels/orphan.txt", "0 0.5 0.5 0.1 0.1\n");
        tmp
    }

    fn checked(tmp: &TempDir) -> (LabelSet, HealthReport) {
        let set = labels::read(&tmp.0, LabelFormat::Yolo, &Silent).unwrap();
        let orphans = labels::orphan_labels(&tmp.0, LabelFormat::Yolo).unwrap();
        let report = check(&tmp.0, &set, orphans, "stamp".into(), &Silent).unwrap();
        (set, report)
    }

    #[test]
    fn finds_every_kind_of_problem() {
        let tmp = broken_dataset("find");
        let (_, report) = checked(&tmp);
        let counts = report.counts();

        assert_eq!(report.corrupt_images.len(), 1, "{:?}", report.corrupt_images);
        assert_eq!(report.corrupt_images[0].image.path, "valid/images/broken.png");
        assert_eq!(report.orphan_labels, ["valid/labels/orphan.txt"]);
        assert_eq!(counts.degenerate_boxes, 1);
        assert_eq!(counts.out_of_bounds, 1, "dog box at x 0.9 ± 0.2 overshoots");
        assert_eq!(counts.unknown_classes, 1, "class 5 with 2 names");
        assert_eq!(report.unlabeled_images.iter().map(|i| i.path.as_str()).collect::<Vec<_>>(), ["train/images/b.png"]);

        assert_eq!((counts.exact_groups, counts.leak_groups), (1, 1));
        let group = &report.duplicates[0];
        assert!(group.exact);
        assert_eq!(group.images[0].split, Some(Split::Train), "train copy first, so it's the one kept");
        assert_eq!(counts.near_groups, 0, "a and b look different");
    }

    #[test]
    fn applies_fixes() {
        let tmp = broken_dataset("fix");
        let (mut set, report) = checked(&tmp);
        let fixes = HealthFixes::recommended(&report.counts());
        let summary = apply_fixes(&tmp.0, &mut set, &report, &fixes);

        let paths: Vec<String> = set.images.iter().map(|i| rel_path(&tmp.0, &i.file)).collect();
        assert_eq!(paths, ["train/images/a.png", "train/images/b.png"], "corrupt image and test copy gone");
        assert_eq!(summary.removed_images, 2);
        assert_eq!((summary.dropped_objects, summary.clipped_objects), (2, 1));

        let a = &set.images[0];
        assert_eq!(a.objects.len(), 2);
        let dog = a.objects[1].bbox;
        assert!(dog.x + dog.w <= 64.0 + 1e-9, "clipped to the image: {dog:?}");
        assert_eq!(set.classes, ["cat", "dog"]);

        // Written out and checked again: clean except b having no labels.
        let out = TempDir::new("fix-out");
        labels::write(&set, &out.0, LabelFormat::Yolo, &Silent).unwrap();
        let (_, again) = checked(&out);
        assert_eq!(again.counts().problems(), 0, "{:?}", again.counts());
    }

    #[test]
    fn finds_look_alikes() {
        let tmp = TempDir::new("near");
        tmp.png("a.png", 2);
        // Same picture saved as JPEG: different bytes, same look.
        image::open(tmp.0.join("a.png")).unwrap().to_rgb8().save(tmp.0.join("a-copy.jpg")).unwrap();
        tmp.png("other.png", 3);
        tmp.write("data.yaml", "names: ['x']\n");
        let (_, report) = checked(&tmp);
        let near: Vec<_> = report.duplicates.iter().filter(|g| !g.exact).collect();
        assert_eq!(near.len(), 1, "{:?}", report.duplicates);
        assert_eq!(near[0].images.len(), 2);
    }

    #[test]
    fn box_rules() {
        assert!(degenerate(&BBox { x: 0.0, y: 0.0, w: 0.0, h: 5.0 }).is_some());
        assert!(degenerate(&BBox { x: 0.0, y: 0.0, w: 0.5, h: 0.5 }).is_some());
        assert!(degenerate(&BBox { x: 0.0, y: 0.0, w: 2.0, h: 2.0 }).is_none());
        assert_eq!(overshoot(&BBox { x: -0.3, y: 0.0, w: 10.0, h: 10.0 }, 10, 10), None, "rounding tolerated");
        assert_eq!(overshoot(&BBox { x: 5.0, y: 0.0, w: 10.0, h: 10.0 }, 10, 10), Some(5.0));
        assert_eq!(clip(BBox { x: -2.0, y: 5.0, w: 6.0, h: 10.0 }, 10, 10), BBox { x: 0.0, y: 5.0, w: 4.0, h: 5.0 });
    }
}
