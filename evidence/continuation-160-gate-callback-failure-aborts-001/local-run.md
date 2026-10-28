# CONTINUATION-160 — échec du callback de gate et arrêt du processus en attente

Date : 26 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD indiqué dans `toolchain-and-head.txt`, dépôt déjà fortement modifié. Aucun état existant n’a été remplacé.

## Vérification

Commande ciblée : voir `commands.txt`. Le test de succès de la barrière et le nouveau test de refus passent ensemble : **2 tests réussis**. `py_compile` du module et `git diff --check` passent.

Le nouveau cas lance un processus Python de stage qui publie son PID puis signale qu’il attend à la barrière. Le callback représentant le vérificateur lève une exception. Le lanceur rend le code 125, n’écrit pas le fichier de libération et le signal `kill(pid, 0)` retourne `ProcessLookupError` après le retour du lanceur. Le test montre que l’échec du callback entraîne l’arrêt de ce processus de stage et ne se transforme pas en libération positive.

## Portée et limites

C’est un test processus local du lanceur Python avec un enfant synthétique, pas le binaire MCP réel, pas une instance Forgejo et pas une fusion. Il ne vérifie pas à lui seul que le pipeline ne démarre aucune étape PR dans chaque chemin; le code courant traite le code non nul de l’étape comme échec avant de parser son résultat. Les tests d’annulation du superviseur MCP restent des preuves distinctes. Aucun LLM/AgentOS, réseau externe, coût, secret ou données privées n’a été utilisé. Aucun changement de runtime Rust n’a été fait.

Les sources et documents liés sont identifiés par SHA-256 dans `source-sha256.txt`. Le préfixe des manifestes avant append est dans `manifest-prefix-before.json`; le script append-only et ses rapports vérifient les index local, worktree et projet. Le projet demeure incomplet.
