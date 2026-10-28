# SANDBOX-RESOURCE-SCOPE-001 — plafonds locaux du vérifieur

Date : 24 octobre 2026. Portée : le runner Rust de `MISSION-012` dans le worktree `.runtime/webhook-005-workspace`. HEAD examiné : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. `src/mission_012_host.rs` est non suivi par Git dans ce worktree; son SHA-256 pour le run final est `432710bb62946082ba945dd48f150ac11a1ed99b65902ca2104d848b33ba81e1`. Le binaire MCP final est lié au SHA-256 `04dd9c5f53f1334da50b204f662e86b1970894e0bdb72f0602aad5e763a01b55` dans la provenance du run 044.

## Changement et modèle d’exécution

`run_sandbox_test` lance maintenant Bubblewrap dans une portée transitoire `systemd --user` et échoue fermé si cette portée ne peut pas être créée. Le runner transmet seulement le bus Unix de l’utilisateur courant après avoir vérifié `/run/user/<uid>` (propriétaire courant, répertoire privé sans accès groupe/autres) et son socket `bus` (socket Unix détenu par le même UID). Il n’utilise pas de `DBUS_SESSION_BUS_ADDRESS` ambiant.

Chaque invocation `cargo.test` reçoit les plafonds suivants : cgroup mémoire à 1 Gio, 64 tâches, débit CPU à 200 % et `RuntimeMaxSec` égal au délai du runner arrondi à la seconde supérieure. Le délai normal de MISSION-012 est 120 s; le quota CPU limite donc une invocation à au plus 240 secondes-CPU si elle peut occuper deux cœurs pendant toute cette durée. Le watchdog hôte garde son propre délai et tue le groupe de processus. `TimeoutStopSec` vaut 1 s. Les limites `prlimit` déjà présentes restent : 120 s par processus pour le CPU, espace d’adressage de 4 Gio, 32 processus par UID, 128 descripteurs et 64 Mio par fichier.

Les sorties de compilation ont quitté le système de fichiers hôte : `/target` est un tmpfs de 256 Mio et `/tmp` un tmpfs de 128 Mio. `target_root` reste validé contre les recouvrements de chemins, mais les fichiers candidats n’y sont plus écrits. Les tests vérifient que ce répertoire hôte reste vide après l’exécution.

Ce réglage est un profil pour le petit vérifieur MISSION-012, pas une limite générique adaptée à tout dépôt Rust. Il s’applique à une invocation de test à la fois; il n’existe toujours pas de comptabilité CPU cumulative commune à toutes les invocations d’une mission. Les appels successifs peuvent donc consommer plusieurs fois le plafond par invocation. L’instance Forgejo, le récepteur webhook et l’agent local ne sont pas dans ce cgroup de vérification; ils partagent encore l’UID de l’utilisateur.

## Vérification

Le test ajouté a d’abord échoué sur l’ancien runner : `df` voyait `982819848192` octets pour le bind `/target` et `65296912384` octets pour le tmpfs `/tmp`; le build script refusait les deux tailles supérieures aux plafonds. Après correction, le build script observe exactement `[268435456, 134217728]` octets.

Commande ciblée : `cargo test --offline --locked --bin mission-012-mcp-host host::tests:: -- --nocapture --test-threads=1 --include-ignored`. Résultat : 13 réussis, 0 échec. Elle inclut le test d’intégration normalement ignoré, qui lit les fichiers cgroup et vérifie `memory.max=1073741824`, `pids.max=64`, `cpu.max=200000 100000`. Le même test laisse expirer un scope d’une seconde après avoir lancé `setsid sleep 30`; le scope tue le processus détaché et son entrée `/proc/<pid>` disparaît.

Suite complète : `cargo test --offline --locked --all-targets --quiet -- --test-threads=1`, code de sortie 0; 33 groupes, 413 réussis, 0 échec, 20 ignorés. `cargo fmt --all -- --check` passe. Les sorties de commande, avec code de sortie, sont [la suite ciblée](sandbox-resource-scope-001-focused.log) et [la suite all-targets](sandbox-resource-scope-001-all-targets.log).

Le premier replay composé 042 s’est arrêté avant la PR. Il a montré que le superviseur fournit volontairement un environnement MCP sans `XDG_RUNTIME_DIR`; dans ce contexte, `systemd-run --user` renvoyait `Failed to connect to bus: No medium found`. Après ajout de la découverte bornée du socket Unix local, les replays 043 puis 044 ont réussi. L’état de diagnostic 042 et ses secrets restent dans `/tmp/mission012-composed-adapter-042`; ils n’ont pas été copiés dans les preuves.

## Parcours Forgejo après correction

Le run 044 reprend l’événement Forgejo v17 `issues/opened`, l’admission versionnée, l’hôte MCP, le changement de `src/lib.rs`, la PR #2, le vérifieur indépendant puis l’autorité de fusion Rust. Sur le candidat, le contrôle d’acceptation passe; l’autorité refuse le merge avant revue et celui du reviewer non autorisé (HTTP 405 chacun), puis fusionne le candidat `f856b812823c9c1b3be54d1a8493b2afa4fe8e58` par le commit `8d7bf3668e9655791cbe5e981910db51a88d9003`. La relecture Forgejo confirme `main` sur ce commit et l’issue fermée. Les trois notifications PR ont obtenu HTTP 202 et sont rapprochées de l’état courant; le hook est ensuite désactivé et le récepteur arrêté, sans listener restant.

Rapport machine et reçus : [résultat composé 044](mission-012-live-agent-webhook-local-adapter-044/composed-run.json), [provenance d’admission](mission-012-live-agent-webhook-local-adapter-044-admission/provenance.json), manifestes [du run](mission-012-live-agent-webhook-local-adapter-044/SHA256SUMS) et [de l’admission](mission-012-live-agent-webhook-local-adapter-044-admission/SHA256SUMS). Les deux `sha256sum --check SHA256SUMS` passent.

L’agent est `local-test-adapter` : aucun LLM n’a été appelé; son coût déclaré 0 USD n’est pas une comparaison de coûts. Ce seul parcours n’établit aucun gain de qualité, coût, délai ou intervention humaine. Le binaire Forgejo est identifié comme version 17.0.0, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; la provenance de sa compilation depuis le tag officiel n’est toujours pas établie. Aucun changement de production, publication, fusion Git du worktree, suppression ou choix de licence n’a été effectué.
