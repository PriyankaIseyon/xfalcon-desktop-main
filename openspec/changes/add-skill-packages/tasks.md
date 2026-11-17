## Validation

- [x] 1. A skill-creator `.skill` (`<name>/SKILL.md`, DEFLATE) imports with every bundled file stored byte-for-byte (`a_skill_creator_package_imports_with_every_bundled_file`).
- [x] 2. A root-layout, stored-method package with directory entries imports the same way (`a_package_with_skill_md_at_its_root_imports_the_same_way`).
- [x] 3. A plain-text `.skill` still imports (`a_plain_text_skill_file_still_imports`, `a_chosen_folder_is_searched_including_its_subfolders`).
- [x] 4. macOS resource forks and packaging noise are left out (`a_macos_zip_leaves_its_resource_forks_behind`, `a_skill_folder_imports_with_its_files_and_its_references_are_not_skills`).
- [x] 5. A zip of packages imports every package with its files (`a_zip_of_packages_imports_every_package_with_its_files`).
- [x] 6. A named `SKILL.md` brings its folder only when the folder is the skill (`a_named_skill_md_brings_its_folder_when_the_folder_is_the_skill`, `a_named_skill_md_in_an_unrelated_folder_imports_alone`).
- [x] 7. A re-import replaces the whole folder, keeps the disabled state, and leaves nothing staged (`re_importing_a_package_drops_the_files_its_new_version_removed`).
- [x] 8. Non-portable, traversal-shaped, and oversized bundles are refused or contained (`a_bundled_file_that_climbs_out_of_the_skill_fails_the_skill`, `a_bundle_larger_than_a_skill_may_be_is_refused_whole`, `a_zip_entry_that_would_escape_the_root_lands_under_its_slug_like_any_other`).
- [x] 9. The model sees bundled files, reads one by path, pages long text exactly, and gets a named reason for every miss (`dispatch_tests::*`, `a_bundled_file_is_read_by_the_path_its_instructions_use`).
- [x] 10. `allowed-tools` string forms and a byte-order mark parse (`allowed_tools_is_read_in_every_spelling_skills_use`, `a_byte_order_mark_does_not_hide_the_frontmatter`).
- [x] 11. Settings shows the bundled-file count, and the contract snapshot and generated TypeScript agree (`SkillsSection.test.tsx`, `contract_snapshots`).
- [x] 12. A bundle whose paths clash on a case-insensitive disk is refused whole (`a_bundle_whose_names_clash_on_a_case_blind_disk_is_refused`).
- [x] 13. Paged bundled-file reads join exactly across multi-byte boundaries (`a_long_bundled_file_is_read_in_pages_that_join_exactly`, against the paged reader).
- [ ] 14. Import refuses a skill past the 256-skill library bound (no test yet).
- [x] 15. Each account sees and runs with only its own skills (`each_account_sees_and_runs_with_only_its_own_skills`).
- [x] 16. The device-wide library moves whole, with its enabled states, into exactly one account (`the_device_wide_library_moves_whole_into_the_first_account_to_open_one`).
- [x] 17. A user who asks which skills and tools they have gets them listed, contents still protected (`the_user_can_be_told_which_skills_and_tools_they_have`, `nothing_to_list_adds_no_catalog`).
- [x] 18. The tool inventory lists each tool under its real server by its server-side name, not its call alias (`an_mcp_tool_is_listed_under_its_server_by_the_name_the_server_gave_it`).
