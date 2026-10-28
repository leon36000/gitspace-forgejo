# CONTINUATION-110 — protocole de capacité superviseur dans le gate

Date : 2026-10-26. Base Git examinée : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié; aucune réinitialisation, suppression, synchronisation, publication ou modification d’un service externe n’a été faite. La tranche touche le nouveau module `src/supervisor_capability.rs`, son export `src/lib.rs`, le chemin d’admission merge de `src/bin/forgejo-reconcile-local.rs`, et les décisions courantes dans `docs/checkpoint.md` et `docs/architecture.md`. Le HEAD n’a pas changé.

## Changement

Le profil `local_single_writer_sandbox` n’accepte plus un booléen du binaire pour représenter l’attestation. Il consomme un descripteur `--supervisor-capability-fd` à usage unique avant de lire le credential ou les entrées de fusion. Le protocole Linux exige un socket AF_UNIX `SOCK_SEQPACKET`, une seule trame JSON strictement typée de 8192 octets au plus, une expiration `CLOCK_BOOTTIME` au plus 120 secondes, un nonce de 64 caractères hexadécimaux, et la liaison à la cible compilée (profil, origine, dépôt, PR, branche `main`), au peer noyau `SO_PEERCRED`, et aux PID/temps de démarrage/namespaces PID et réseau du superviseur, du gate et du processus Forgejo indiqué. Le gate relit l’identité autour des symlinks `/proc`, refuse les paquets en attente supplémentaires et garde le canal vivant; il refait les contrôles dans le hook appelé après les dernières lectures Forgejo et juste avant le POST de merge.

Le résultat du gate ajoute les bindings non secrets, l’empreinte SHA-256 du nonce et l’indicateur du contrôle pré-POST. Il affiche explicitement `supervisor_policy_independently_verified=false`; le champ d’exclusion des écrivains ne transforme pas le reçu en preuve de correction. Le profil `forgejo_v17_multiwriter` reste refusé avant les credentials, car Forgejo v17 ne requiert pas la précondition atomique de base sur tous ses chemins.

## Vérifications exécutées

Sur Linux aarch64, Rust/Cargo 1.95.0, dépendances locales et verrouillées :

- `cargo test --offline --locked --all-targets` : code 0; 35 groupes, 610 réussites, 0 échec, 101 ignorés. Le log complet est [all-targets-final.log](all-targets-final.log). `all-targets.log` est un run intermédiaire antérieur à l’ajout du test gate positif; le fichier `all-targets-final.log` fait autorité.
- Les tests ajoutés couvrent le handshake valide puis le recheck, le scope différent, l’expiration, le peer PID différent, PID/namespace Forgejo incorrect, un flux `SOCK_STREAM`, JSON mal formé, seconde trame/rejeu sur le même canal, trame surdimensionnée et fermeture du superviseur.
- Deux tests du CLI vérifient le refus d’une capacité absente ou d’un descripteur invalide avant lecture du pipe de credential; les octets synthétiques restent disponibles et inchangés.
- `live_supervisor_capability_reaches_fixture_merge_and_is_bound_in_audit_output` exécute le gate de merge complet contre un serveur HTTP de fixture, observe exactement un POST et confirme le lien vers la cible ainsi que le check pré-POST.
- `cargo check --offline --locked --bin forgejo-reconcile-local`, `cargo fmt --all -- --check` et `git diff --check` terminent avec code 0.
- `cargo clippy --offline --locked --bin forgejo-reconcile-local` termine avec code 0, mais produit des avertissements déjà présents dans `agent_runtime.rs`, `forgejo.rs`, `mission_admission.rs`, `webhook/reconcile.rs`, `lib.rs` et un `useless_format` dans le parseur de profil existant. Le nouveau module n’apparaît pas dans les avertissements. Sortie : [clippy.log](clippy.log).

Les sorties, outil et empreintes de sources sont joints; commandes exactes : [command.txt](command.txt), environnement : [toolchain.txt](toolchain.txt), hashes : [source-sha256.json](source-sha256.json).

Les manifestes globaux ont été prolongés uniquement en ajoutant les hashes de ce répertoire et des deux documents canoniques modifiés. `manifest-prefix-before.json` fixe les hashes et longueurs de leurs préfixes préexistants; `append_manifest_entries.py` vérifie ces préfixes avant d’ajouter, et `verify_append_only_indexes.py` vérifie le répertoire, les hashes courants et les deux préfixes après append.

## Portée et réserves

Le succès du serveur HTTP est une fixture dans le même processus Rust que le gate. Le PID « Forgejo » de ce test est donc le processus de test; ce n’est ni un Forgejo live ni une preuve de namespaces séparés. Le test démontre l’intégration du lecteur jusqu’au hook pré-POST dans le gate, pas l’exclusion de tous les writers ni un parcours de fusion réel après le nouveau protocole.

Aucun trusted launcher ne crée encore la capacité, ne démarre le Forgejo jetable et le gate dans le namespace commun, ne prouve qu’aucun autre writer n’y accède, ou ne maintient le peer jusqu’à la fin. Le lecteur lie le PID noyau au créateur de la socket et aux identités de processus déclarées; il n’identifie pas le fichier exécutable du peer ni sa politique. Un acteur host du même UID peut construire sa propre socket. Un superviseur qui possède le credential, choisit le processus Forgejo et l’isolation reste un point de confiance; root, un noyau compromis, un processus privilégié ou une évasion ne sont pas couverts. Les profils single-writer et multi-writer n’ont donc toujours pas de parcours de fusion utilisable sans un lanceur qui impose ce modèle et en fournit la capacité. Aucun modèle LLM, coût, secret réel ou service externe n’a été utilisé.

## Suite

Construire et éprouver un lanceur local/rootless uniquement dans le bac à sable : lancer le Forgejo jetable puis le gate dans le namespace réseau commun sans namespace PID séparé, établir les PID/starttime/namespaces après démarrage, transmettre seulement le FD de capacité au gate et le credential uniquement à celui-ci, et garder le peer ouvert jusqu’au readback. L’agent doit rester dans son propre namespace PID + réseau, sans FD/credential; tester les tentatives API, Web et `git-receive-pack`, puis tuer/reprendre le superviseur et confirmer que le gate refuse avant le POST. Avant ce parcours, vérifier le binaire/version exacte du Forgejo local et ne jamais rediriger vers la production.
