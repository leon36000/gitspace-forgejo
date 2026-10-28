# WEBHOOK-STORAGE-HARDENING-001 — stockage privé du webhook

Date : 2026-10-22, heure de Montréal. HEAD examiné : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Le checkout contient de nombreux changements locaux antérieurs non commités; ils ont été préservés.

## Changements

- `EventJournal::append_locked` crée maintenant un journal en mode `0600`. Le test d’intégration lance le véritable binaire `forgejo-webhook-local` avec umask `022`, reçoit un webhook HMAC synthétique, vérifie le mode du fichier, puis redémarre le receiver sur le journal créé.
- `read_secret` ouvre une seule fois le fichier avec `O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC`, vérifie le type, l’UID, les liens et les permissions sur le descripteur ouvert, puis lit ce même descripteur sur une limite de 4097 octets.
- Le receiver exige un répertoire de journal appartenant à l’UID courant, de type répertoire et de mode exact `0700`. Un journal existant doit être régulier, appartenir à l’UID courant, avoir un seul lien et être en mode exact `0600`. Les cas invalides sont refusés avant le bind.

## Preuves

Le nouveau test rouge a échoué sous umask `022` avant le changement : le mode observé était `0644` (`420` décimal), alors que le contrat exigeait `0600` (`384`). La suite all-targets pré-correctif avait échoué dans le même test de processus au redémarrage après cinq tentatives; le test forcé rend ce défaut observable. Après correction, le même test passe et le receiver redémarre avec succès sur le journal créé : [rouge](webhook-storage-hardening-001-private-journal-red.log), [vert](webhook-storage-hardening-001-private-journal-green.log).

La vérification complète, sur l’état courant, passe 27 exécutables Cargo : 258 réussis, 0 échec, 7 ignorés. Les sept cas ignorés comprennent cinq tests CLI qui exigent `bwrap --unshare-net` et deux helpers lancés par leurs tests parents. Journaux : [all-targets](webhook-storage-hardening-001-all-targets-final.log), [receiver unit](webhook-storage-hardening-001-secret-tests-final.log), [refus de stockage](webhook-storage-hardening-001-storage-process-tests-final.log), [processus receiver](webhook-storage-hardening-001-receiver-e2e-final.log).

Clippy all-targets passe avec les trois allowances historiques (`too_many_arguments`, `bool_assert_comparison`, `manual_div_ceil`); le build release all-targets, rustfmt, `git diff --check`, le readback Cargo et les cinq tests de garde de licence passent. Résultats : [Clippy](webhook-storage-hardening-001-clippy-final.log), [release](webhook-storage-hardening-001-release-build-final.log), [rustfmt](webhook-storage-hardening-001-rustfmt-final.log), [diff-check](webhook-storage-hardening-001-diff-check-final.log), [readback licence](webhook-storage-hardening-001-cargo-license-final.json), [tests licence](webhook-storage-hardening-001-cargo-license-tests-final.log).

Une revue indépendante en lecture seule a confirmé que le mode de création et l’ouverture du secret couvrent les deux défauts signalés. Elle relève que `O_NOFOLLOW` porte sur le dernier composant du chemin; les parents sont traversés normalement. Les métadonnées du fichier effectivement ouvert restent contrôlées. Voir [revue](webhook-storage-hardening-001-review.md).

Le manifeste SHA-256 recalcule et vérifie les sources, documents et journaux de ce lot : [manifeste](webhook-storage-hardening-001-sha256.txt).

## Limites et suite

Le parcours est local et synthétique : aucun secret réel, webhook Forgejo, modèle, agent LLM ou admission via `forgejo-mission-local` n’a été utilisé. Ce durcissement ne change pas le fait que le HMAC brut n’est pas conservé ni revérifié lors d’une relecture du journal. Un processus du même UID peut toujours altérer les fichiers; un worktree n’isole pas ses privilèges. Aucun gain produit n’est mesuré.

Ne pas relier l’inbox à l’admission avant de définir une frontière d’autorité pour l’UID du journal et la provenance Forgejo, et de décider comment traiter les anciens fichiers de journal dont le mode n’est pas `0600`. Aucune instance Forgejo n’était disponible dans ce parcours; rien n’a été publié ni fusionné.
