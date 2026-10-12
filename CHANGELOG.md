# Changelog

What changed in each release of the launcher, newest first. The release workflow copies the section of the released version into the notes of the GitHub release and into `latest.json`, where the launcher shows it as plain text before it updates itself. A version without a section here is not released.

## 0.1.0

The first release: install OA Mod for Golf With Your Friends, keep it current, start the game, and put the game back to clean.

- Finds the game through Steam, or takes a folder you choose.
- Install downloads BepInEx and OA Mod, checks both against a signed release list, and puts them into the game folder. Update replaces only the mod.
- Play starts the game through Steam. The first start after an install takes a few minutes, and the launcher says so.
- Remove mod deletes only OA Mod, or everything BepInEx brought. Other plugins are copied to a backup folder first, and files the launcher does not recognise stay unless you ask for them.
- Nothing in the game folder is changed while the game runs or while Steam is busy with it.
- The launcher updates itself, from a signed installer.
