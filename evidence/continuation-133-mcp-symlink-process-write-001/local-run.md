# CONTINUATION-133 — refus de liens symboliques par le processus MCP

## Objectif

CONTINUATION-132 vérifiait directement la primitive d’écriture host. Ce lot exerce la frontière publique `workspace_write_text` dans le binaire MCP réel après son démarrage, lorsque les chemins du candidat changent après la validation initiale. Aucun code de Forgejo, GitSpace, AgentOS ou modèle n’est utilisé.

## Scénario

Le test `tests/mission_012_mcp_lifecycle.rs::mcp_process_refuses_source_and_directory_symlinks_injected_after_initialization` lance deux processus host sur des fixtures privées. Chaque processus initialise MCP, exécute d’abord `cargo_test` et observe l’échec de base attendu, puis le harness remplace soit `candidate/src/lib.rs`, soit `candidate/src` par un lien vers un chemin extérieur au candidat et appelle l’outil MCP d’écriture.

Pour le lien de fichier, la lecture sûre du candidat refuse `tool_file_not_regular_or_too_large`. Pour le lien de répertoire, la fixture extérieure contient exactement le source de base, de sorte que le hash pré-écriture ne provoque pas de refus anticipé; `write_source` atteint `openat(..., O_NOFOLLOW)` et refuse `tool_workspace_invalid`. Dans les deux cas le lien est conservé, les octets de la cible restent identiques, le reçu de l’outil contient le code de refus, la finalisation du candidat est `refused` et le processus quitte avec le code 2.

Une première itération de test a appelé l’écriture avant le contrôle de base et a reçu le refus attendu `baseline_not_verified`; le harness a été corrigé pour suivre le protocole. La suivante attendait à tort `tool_workspace_invalid` pour le lien de fichier; le code observé est le refus plus tôt de la lecture régulière. Ce sont des corrections d’assertions/harness, pas des défauts runtime. Le test vert final vérifie les deux codes distincts.

## Résultat

- Le test processus ciblé passe : 1 test, 0 échec, 0 ignoré, deux sous-cas host.
- `cargo fmt --all -- --check` passe.
- `git diff --check` passe.
- Les commandes, versions, sortie et empreintes courantes sont jointes. Le HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` n’inclut pas ces fichiers locaux : le worktree est fortement modifié et certains fichiers source/test sont non suivis. Les hashes identifient les octets réellement exercés.

## Portée et limites

Test de processus local Linux avec Bubblewrap; mutation séquentielle par le harness entre l’initialisation et l’appel outil. Il confirme que le chemin MCP refuse ces deux substitutions et que le writer ne modifie pas leur cible. Il ne démontre pas une course hostile concurrente à chaque syscall, une résistance à un processus privilégié, une séparation multi-UID, l’absence d’évasion noyau, un comportement Forgejo live, un LLM/AgentOS, ni un gain comparatif. Aucun runtime de production n’a été changé. Le lot ne ferme pas les critères de complétion du projet.
