To select this strategy, close the game and set config.json to:

{ "strategy": "standard-malevolent" }

Save the file and restart the game. Keep your existing history when updating or switching strategies.

This version responds more strongly to an option whose tracked window contains a valid HP-to-zero transition. Malevolent strategies favor it; benevolent strategies avoid it. Part thresholds do not add rewards. Set zero_hp_bonus in params.json (default 1.0; 0 disables the bonus). This is an observation, not guaranteed cart recognition. Version 2 rebuilds its own state from available event history; old version 1 profiles are retained.
