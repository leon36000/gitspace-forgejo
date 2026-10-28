# MISSION-012 — composition locale des processus et vérification

Date : 23 octobre 2026. Exécution locale dans le worktree d’intégration, sans déploiement.

## État exact des entrées

- Worktree : `.runtime/webhook-005-workspace`; `HEAD` observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree est sale et le runner compile les fichiers de travail mis en scène, pas un arbre supposé identique à `HEAD`.
- Empreinte SHA-256 de l’arbre Cargo mis en scène : `e734f5b159c85403af9e869f0654ca69859a1c5d0f2dd42cbf1e8d61790dd59a` (nombre d’entrées, types, permissions, noms relatifs, longueurs et contenus, encodage versionné).
- Runner `scripts/run_forgejo_mission_delivery_process_e2e.py` : `0a41a99f5441c982019c81d56a4edd3ca9c0033dae3f6748beea42eba1d90194`.
- `Cargo.lock` : `bf599559dd11ff21ecc4754aa8a693966b4a9ca401153d7d7caa1f18d8995fcc`; registre filtré pour les crates verrouillées : `a5ea9d0f4ab9d3e50cc22b0b22e0c5b93274462e61193b074e0a2173e5e46e30`.
- Environnement journalisé : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`, Bubblewrap `0.9.0`, Linux aarch64 avec glibc 2.39.
- Politique fixture MISSION-012 v4 : SHA-256 `e23907b5bd6b2c0f342c88ddcfa9f9a9be98065adcd24f9f7299dc796b5739eb`.

Ces empreintes décrivent les entrées réellement compilées par ce run. Aucun commit, push, publication ou changement d’instance n’a eu lieu.

## Commandes et résultat observé

```text
python3 -m py_compile scripts/run_forgejo_mission_delivery_process_e2e.py   # exit 0
python3 scripts/run_forgejo_mission_delivery_process_e2e.py                 # exit 0
```

Les trois tests de processus ciblés passent dans [le journal final](mission-012-composed-process-e2e-final-bounded.log), SHA-256 `09b39a885b71867ade337e48885ea2d1a0c8d033cd06e012db9ea39fa5e15748`. Le contrôle ponctuel des chemins de cache refuse les liens au niveau racine, parent et fichier, et accepte un fichier régulier; sa trace est `mission-012-composed-process-e2e-cache-link-check.log`, SHA-256 `b61eca23e6caee8321df91b4825f82dbb252c29596f7046d4dc811bd4f3ddc81`. Le contrôle du plafond de sortie tue une commande qui dépasse 512 Kio et maintient la capture sous la limite; trace `mission-012-composed-process-e2e-runner-guards.log`, SHA-256 `b10040d5c222a4a948211edda763343ba89fce0de7b4e9e7d0ed6ee163b09ec4`.

1. Le vrai processus CLI refuse l’issue fermée (`mission_issue_closed`), puis admet l’issue rouverte par la fixture HTTP. Le doublon et une autre livraison réutilisent `admission-41`, génération 1; le scénario observe 31 requêtes HTTP à la fixture.
2. Le CLI refuse une livraison inconnue sans créer d’état runtime.
3. Le scénario MISSION-012 admet une issue signée et rejouée idempotemment; lie le contexte à `0c5edf83181c8c05a7e100c07c170f5dc3b08225`; fait échouer un critère d’acceptation avant la correction et le réussir ensuite; refuse la réécriture des tests protégés et l’auto-publication d’un statut. Le processus de vérification distinct rend `pass` pour le candidat `89ec5dbe07f60397c1a57a4a9b22db99c59c0ae3`; Git avance ensuite la branche locale temporaire en fast-forward et l’arbre fusionné égale celui du candidat.

Le test lance le vérificateur dans un processus séparé, mais celui-ci réutilise un module Rust partagé avec l’hôte MCP; son implémentation n’est donc pas indépendante. Le scénario utilise un dépôt et une fixture synthétiques, MCP scripté et une fusion Git locale. Il indique lui-même que l’API Forgejo, la création d’une PR et un LLM ne sont pas exercés. Le run du 22 octobre avec PR Forgejo locale #4 et Claude Opus 5.5/high est une preuve distincte [documentée ici](mission-012-local-run.md), pas le même parcours bout en bout.

## Confinement du runner

Le runner ne met en scène que `Cargo.toml`, `Cargo.lock`, `src`, `tests` et `build.rs`; il refuse les liens symboliques et fichiers spéciaux, et borne l’entrée à 256 Mio et 50 000 fichiers. Il copie uniquement les archives `.crate` des 24 paquets du registre présents dans le lockfile et les entrées sparse-index nécessaires; chaque archive est vérifiée contre son checksum `Cargo.lock`, les composants de chemins de cache sont vérifiés sans lien symbolique, et le réseau Cargo est désactivé.

Bubblewrap monte `/usr`, le sysroot Rust et le code en lecture seule; le home hôte n’est pas monté. Les namespaces réseau et PID sont séparés. Les tmpfs internes sont limités à 64 Mio pour `/home`, 128 Mio pour `/tmp`, 1 Gio pour `/build/target`, 64 Mio pour `.runtime` en mode `0700`, et 128 Mio pour `/artifacts`. Cargo copie le cache filtré dans son home temporaire et décompresse les crates dans `/tmp` borné. `prlimit` applique par processus 240 s CPU, 4 Gio d’espace d’adressage, 384 processus, 128 descripteurs et 64 Mio par fichier; le runner borne chaque commande à 300 s et les sorties des tests à 512 Kio. Seuls les exécutables nommés sortent du build; le flux d’artefacts est plafonné à 128 Mio. Staging source/cache et artefacts hôtes sont plafonnés respectivement à 256/128/128 Mio (environ 512 Mio cumulés, hors métadonnées de filesystem).

Ce profil réduit les accès fichiers et réseau des scripts de build et tests; il n’impose aucun quota cgroup mémoire/CPU agrégé et ne protège pas contre une échappée noyau, un administrateur ou un processus du même UID. La validation de chemin puis la copie n’est pas atomique : un processus local concurrent pourrait remplacer un parent entre les deux. Les build scripts et macros exécutés partagent aussi l’espace de compilation et peuvent modifier `/build` avant l’export des binaires; la provenance de l’archive crate ne prouve pas la correction ou l’intégrité du binaire de test. Il faudrait une séparation de build/vérification plus forte pour un modèle incluant des dépendances volontairement hostiles. Le runner reste un harnais local, pas une frontière de sécurité de production.

## Couverture restante

Le scénario vérifie des processus réels de notre code, une fixture HTTP synthétique, les refus de capacités, un critère rouge puis vert, un processus de vérification séparé à implémentation partagée et une fusion Git locale. Il ne vérifie pas l’authenticité auprès d’une instance Forgejo, l’API de PR, les protections/statuts Forgejo, une autorité de fusion Forgejo, AgentOS/RAG, un LLM dans ce nouveau run ou une CI conventionnelle de référence. La preuve antérieure PR+LLM et ce test composé restent complémentaires, sans constituer un test joint unique. Aucun gain de taux de réussite, de temps, d’interventions humaines, de tokens ou de coût n’est mesuré; un essai antérieur `n=1` ne permet pas d’inférer une amélioration générale.

Revue read-only indépendante du runner final : les écarts précédents d’empreinte, de hash de runner et de plafond de sortie sont corrigés. Elle conserve explicitement les limites TOCTOU et d’intégrité des artefacts de build décrites ci-dessus; elle confirme que Forgejo réel/PR/LLM ne sont pas exercés dans ce nouveau run.

Surveillance Claude Code non intrusive le 23 octobre : PID `868750` toujours vivant (`claude`, état `Ssl+`, aucun enfant visible; durée observée 2 h 44 min). Ni arguments, prompt, environnement, terminal/session ni fichiers n’ont été lus; aucune permission n’a été approuvée et aucun processus n’a été relancé. Le contenu exact de sa tâche et son itération restent inconnus.
