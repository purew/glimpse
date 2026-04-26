//! Viewer model and access-control filtering.

use crate::content::{Post, Site};

/// The entity whose perspective determines which posts are visible.
#[derive(Debug, Clone)]
pub struct Viewer {
    pub groups: Vec<String>,
    pub logged_in: bool,
    pub username: Option<String>,
}

impl Viewer {
    pub fn admin() -> Self {
        Self {
            groups: vec!["admin".to_owned()],
            logged_in: true,
            username: None,
        }
    }

    pub fn public() -> Self {
        Self {
            groups: vec!["public".to_owned()],
            logged_in: false,
            username: None,
        }
    }

    pub fn with_groups(groups: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            groups: groups.into_iter().map(Into::into).collect(),
            logged_in: true,
            username: None,
        }
    }

    pub fn with_groups_and_username(
        groups: impl IntoIterator<Item = impl Into<String>>,
        username: String,
    ) -> Self {
        Self {
            groups: groups.into_iter().map(Into::into).collect(),
            logged_in: true,
            username: Some(username),
        }
    }

    pub fn is_admin(&self) -> bool {
        self.groups.iter().any(|g| g == "admin")
    }

    pub fn can_view(&self, post: &Post) -> bool {
        if self.is_admin() {
            return true;
        }
        if post.is_draft() {
            return false;
        }
        post.access
            .iter()
            .any(|g| g == "public" || self.groups.contains(g))
    }
}

/// Returns an iterator over posts visible to `viewer`, in site order (ascending date).
pub fn visible<'a>(site: &'a Site, viewer: &Viewer) -> impl Iterator<Item = &'a Post> {
    site.posts.iter().filter(move |p| viewer.can_view(p))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Post;
    use std::path::PathBuf;

    fn make_post(access: Vec<&str>) -> Post {
        Post {
            slug: "test".into(),
            title: "Test".into(),
            date: "2025-01-01".into(),
            access: access.into_iter().map(str::to_owned).collect(),
            cover: None,
            body_html: String::new(),
            photo_groups: vec![],
            source_dir: PathBuf::from("."),
        }
    }

    #[test]
    fn admin_sees_drafts() {
        let post = make_post(vec![]);
        assert!(Viewer::admin().can_view(&post));
    }

    #[test]
    fn public_viewer_blocked_from_draft() {
        let post = make_post(vec![]);
        assert!(!Viewer::public().can_view(&post));
    }

    #[test]
    fn public_viewer_sees_public_post() {
        let post = make_post(vec!["public"]);
        assert!(Viewer::public().can_view(&post));
    }

    #[test]
    fn group_member_sees_matching_post() {
        let post = make_post(vec!["family"]);
        assert!(Viewer::with_groups(["family"]).can_view(&post));
    }

    #[test]
    fn group_member_blocked_from_other_group_post() {
        let post = make_post(vec!["family"]);
        assert!(!Viewer::with_groups(["friends"]).can_view(&post));
    }

    #[test]
    fn visible_filters_correctly() {
        let site = Site {
            posts: vec![
                make_post(vec!["family"]),
                make_post(vec![]), // draft
                make_post(vec!["public"]),
            ],
        };

        let family = Viewer::with_groups(["family"]);
        let visible_posts: Vec<_> = visible(&site, &family).collect();
        assert_eq!(visible_posts.len(), 2);

        let admin_posts: Vec<_> = visible(&site, &Viewer::admin()).collect();
        assert_eq!(admin_posts.len(), 3);

        let public_posts: Vec<_> = visible(&site, &Viewer::public()).collect();
        assert_eq!(public_posts.len(), 1);
    }
}
