//! The patch clipboard and PhpStorm patches.

use super::*;

#[gpui::test]
fn shelve_and_unshelve_through_the_patch_clipboard(cx: &mut TestAppContext) {
    use osier_git::ops::patches::ApplyOptions;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    repo.write("a.txt", "two\n");
    repo.write("new.txt", "fresh\n");
    let h = Harness::open(cx, &repo);

    h.act(cx, |osier, window, cx| {
        osier.shelve_named("my shelf".into(), Vec::new(), window, cx)
    });
    cx.run_until_parked();
    let (id, text) = h.read(cx, |osier, _| {
        assert_eq!(osier.patches.len(), 1, "one stored patch");
        let p = &osier.patches[0];
        assert_eq!(p.meta.name, "my shelf");
        assert_eq!(p.meta.files, ["a.txt", "new.txt"]);
        assert_eq!(osier.tab, Tab::Patches);
        (p.id.clone(), p.text().expect("patch text"))
    });
    assert_eq!(repo.read("a.txt"), "one\n", "tracked change discarded");
    assert!(!repo.path().join("new.txt").exists(), "untracked file gone");

    // Unshelve: apply the stored text and delete the entry afterwards.
    h.act(cx, |osier, window, cx| {
        let id = id.clone();
        osier.run_apply(
            text.clone(),
            ApplyOptions::default(),
            "\"my shelf\"".into(),
            Some(std::rc::Rc::new(
                move |this: &mut Osier,
                      window: &mut gpui::Window,
                      cx: &mut gpui::Context<Osier>| {
                    this.delete_stored(id.clone(), window, cx)
                },
            )),
            window,
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(repo.read("a.txt"), "two\n");
    assert_eq!(repo.read("new.txt"), "fresh\n");
    h.read(cx, |osier, _| {
        assert!(osier.patches.is_empty(), "unshelved patch deleted");
        assert_eq!(osier.tab, Tab::Changes);
    });
}

#[gpui::test]
fn phpstorm_patches_apply_and_render(cx: &mut TestAppContext) {
    use gpui_kit::component::diff::DiffFile;
    use osier_git::ops::patches::{ApplyOptions, normalize};
    use osier_git::testing::PHPSTORM_PATCH;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\ntwo\nthree\n", "first");
    let h = Harness::open(cx, &repo);
    // The diff view gets the normalised text; gpui-kit's parser must accept it.
    for text in [
        PHPSTORM_PATCH.to_string(),
        PHPSTORM_PATCH.replace("\t(revision", "    (revision"),
    ] {
        let files = DiffFile::parse(&normalize(&text)).expect("gpui-kit parses it");
        assert_eq!(files.len(), 2);
    }
    let spaced = PHPSTORM_PATCH.replace("\t(revision", "    (revision");
    h.act(cx, |osier, window, cx| {
        osier.run_apply(
            spaced.clone(),
            ApplyOptions::default(),
            "clipboard patch".into(),
            None,
            window,
            cx,
        )
    });
    cx.run_until_parked();
    assert_eq!(repo.read("a.txt"), "one\nTWO\nthree\n");
    assert_eq!(repo.read("notes/new.md"), "# Notes\n-- not a signature\n");
    h.read(cx, |osier, _| assert_eq!(osier.tab, Tab::Changes));
}
