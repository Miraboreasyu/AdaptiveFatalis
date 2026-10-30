# AdaptiveFatalis

A Monster Hunter: World — Iceborne mod that lets Fatalis adapt its move choices across hunts. Choose a helpful or challenging strategy, or keep the original move probabilities.

## Download

Download the mod ZIP from [Releases](https://github.com/Miraboreasyu/AdaptiveFatalis/releases/latest). Use the attached mod ZIP, not GitHub's automatically generated source-code archives.

## Installation

1. Close the game and back up your existing mods.
2. Make sure a compatible nativePC plugin loader is installed. A loader is not included.
3. Extract the ZIP and merge its `nativePC` folder into your Monster Hunter World game folder, alongside `MonsterHunterWorld.exe`.
4. Start the game and wait a few seconds at the title screen before entering a hunt.

The mod includes the files it needs. You do not need to extract game files or run the included helper programs manually. Avoid using other mods that replace Fatalis's AI at the same time.

## Choose a strategy

Close the game, then open:

`nativePC/plugins/FatalisAdaptive/config.json`

Replace its contents with one of these examples. Save the file and restart the game. Strategy names must match exactly.

### Original probabilities — default

```json
{ "strategy": "none" }
```

Keeps the original move probabilities. Hunt history is still recorded, so you can choose an adaptive strategy later.

### Standard helpful

```json
{ "strategy": "standard-benevolent" }
```

Gradually favors moves that have been more favorable to the player.

### Fast helpful

```json
{ "strategy": "fast-benevolent" }
```

The helpful option with stronger, faster changes.

### Standard challenging

```json
{ "strategy": "standard-malevolent" }
```

Gradually favors moves that have been more effective against the player.

### Fast challenging

```json
{ "strategy": "fast-malevolent" }
```

The challenging option with stronger, faster changes.

Changes carry over to later hunts; this is not an instant difficulty switch during combat. Strategies use your recorded history when selected. You do not need to delete your saves or reset the mod when switching strategies.

## Multiplayer

The included strategies exclude newly recorded multiplayer data from learning once another player is detected. You do not need to change your configuration before joining friends. Your existing single-player history is kept.

This protects learning data; it does **not** switch Fatalis back to its original behavior during multiplayer or guarantee multiplayer compatibility. Changes learned before multiplayer was detected are not undone. Third-party strategies may handle multiplayer data differently.

## Updating

Close the game, back up your current mod folder, and merge the new release's `nativePC` folder into the game folder. Keep your existing `register` and `deployment` folders. Do not delete them while upgrading.

The release includes a default configuration using `none`. After copying the update, check `config.json` and select your preferred strategy again.

## Troubleshooting

If the mod does not start, check that your loader is installed and that the strategy name in `config.json` is spelled correctly. Check `nativePC/plugins/FatalisAdaptive/runtime.log` when reporting a problem.

Back up your mods and try the mod in solo play first. Compatibility with every game version or mod combination is not guaranteed.

## Disabling

To stop adaptive changes, set the strategy to `none` and restart the game. To remove the mod, exit the game normally first and keep a backup of its folder. If the game crashed, do not delete the mod's recovery files before the managed game files have been restored.
