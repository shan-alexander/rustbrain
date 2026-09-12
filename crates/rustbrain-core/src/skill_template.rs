//! Embedded rustbrain `SKILL.md` (must match repo-root `SKILL.md`).

/// Agent loop + CLI cookbook written by bootstrap into host harnesses.
#[inline(never)]
pub fn default_skill_md_template() -> &'static str {
    include_str!("templates/SKILL.md")
}
