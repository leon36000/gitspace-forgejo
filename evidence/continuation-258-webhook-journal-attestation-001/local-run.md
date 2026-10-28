# CONTINUATION-258 — attestation facultative du journal webhook

Date : 27 octobre 2026. Base examinée : HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche locale `local/WEBHOOK-005-canonical-links`. L’arbre est fortement modifié par les lots précédents; aucune réinitialisation, suppression ou publication n’a été faite. Le SHA des fichiers concernés est consigné dans `source-sha256.txt`.

## Résultat

Le journal `EventJournal` ajoute une attestation Ed25519 versionnée facultative. Le receiver vérifie d’abord le HMAC Forgejo et le contrat de l’événement, puis signe sous le verrou d’append les champs d’identité, séquence, idempotence, type, payload normalisé et hash précédent. La signature est elle-même incluse dans le hash de la ligne. Les événements historiques omettent le champ en sérialisation et gardent leur calcul de hash précédent.

Le binaire `forgejo-webhook-local` peut recevoir en option une graine privée brute de 32 octets. Le contrôle de fichier refuse symlink, fichier non régulier, propriétaire différent, liens multiples et permissions groupe/autre; l’ouverture utilise `O_NOFOLLOW`, `O_CLOEXEC` et `O_NONBLOCK`. Le tampon temporaire est effacé après construction de la clé. La clé publique peut être affichée par une commande dédiée.

Une API bibliothèque permet de vérifier une attestation avec une clé publique approuvée fournie par l’hôte. Le lecteur strict refuse signature absente, format/algorithme inconnu, clé non approuvée et contenu altéré. La clé publique n’est jamais apprise depuis la ligne qu’elle est censée authentifier.

## Vérification

`cargo test --offline --locked --all-targets` a réussi avec **682 tests passés, 0 échec et 140 ignorés** dans 37 groupes de cibles. Ce run a précédé l’ajout d’une assertion de test supplémentaire portant sur la chaîne précédente; le test HTTP/contrat (3/3) et le test de processus receiver réel (1/1) ont été rejoués après cette assertion. `cargo check --offline --locked --all-targets`, `cargo fmt --all -- --check`, `git diff --check` et les 12 tests du garde Cargo licence passent. Les journaux et la liste exacte des commandes sont adjacents.

Le test contrat appelle le vrai handler HTTP avec une fixture HMAC locale, puis relit et vérifie la ligne avec la clé publique seule. Les refus vérifient mauvaise clé, ligne historique sans attestation, mauvais HMAC sans append, altérations de payload/séquence/hash précédent et signature falsifiée même si la chaîne non signée est recalculée. Le test processus lance le binaire receiver, poste une livraison loopback synthétique et relit la ligne réellement persistée.

`cargo clippy --offline --locked --all-targets -- -D warnings` échoue avec **23 diagnostics** dans les modules existants listés dans `clippy-final.log`; aucune localisation du rapport ne touche la nouvelle logique d’attestation ou le binaire receiver. Cela reste un échec de vérification et ne doit pas être présenté comme Clippy vert.

## Dépendances et licences

Le lockfile comporte 39 paquets externes de registre et huit dépendances directes. Les 39 déclarent une métadonnée `license` ou `license_file`, et un fichier de licence conventionnel est présent dans le cache local pour chacun. Ce contrôle de présence n’est pas une lecture juridique ni une conclusion de compatibilité. `ed25519-dalek` 3.0.0 déclare BSD-3-Clause et MSRV Rust 1.85 selon sa fiche [docs.rs](https://docs.rs/crate/ed25519-dalek/3.0.0); le projet n’a toujours pas choisi sa licence ni évalué ses droits de redistribution (`license=null`, `publish=[]`, droits `not_assessed`). Le lockfile et son hash sont dans `environment.json` et `dependency-license-inventory.json`.

## Limites et prochaine action

**Cette primitive n’est pas encore une barrière d’admission.** Les exécutables `forgejo-reconcile-local` et `forgejo-mission-local` utilisent encore le lecteur de compatibilité; aucun n’exige une clé publique de confiance avant de lire le journal, d’ouvrir un accès réseau ou de demander un credential. Le receiver en mode historique continue d’écrire sans signature. Aucun service Forgejo, PR/merge, gate, LLM ou AgentOS n’a été utilisé dans ce lot.

La clé signe la provenance de la ligne par le processus qui détient cette clé; elle n’atteste pas la correction du logiciel, l’origine Forgejo au-delà du HMAC vérifié à l’ingestion, ni l’absence de vol de clé. Elle ne prévient pas le rollback ou la troncature du journal. Les anciennes lignes n’ont aucune signature; il manque configuration d’épinglage, rotation/révocation, migration, et politique stricte de refus à chaque consommateur. La prochaine tranche doit placer les clés de confiance hors journal et imposer la vérification stricte dans les consommateurs autoritaires avant toute capacité/credential, avec tests de refus et de compatibilité explicites.

Le travail est implémenté et vérifié à l’échelle du handler et du processus local; il n’est pas testé en bout en bout avec Forgejo, l’agent réel, le reconciler ou la fusion. Les critères globaux de mission et la comparaison contrôlée restent ouverts.

## Contrôle des manifestes

Le premier lancement du nouveau vérificateur d’empreintes a trouvé une erreur dans son ordre d’itération (nom/hash inversés), consignée dans `manifest-verification-first-attempt.log`. Le vérificateur a été corrigé; la validation finale recalcule les 32 empreintes locales, compare les 12 sources et préserve les deux préfixes globaux octet pour octet. Le préfixe antérieur de l’index du worktree contient 6 lignes vides et 1 ligne mal formée; celui du projet, 2 lignes vides et aucune mal formée. Elles sont conservées telles quelles en append-only. Résultat exact : `manifest-verification-final.json`.
