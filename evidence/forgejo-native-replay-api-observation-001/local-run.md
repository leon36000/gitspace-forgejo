# FORGEJO-NATIVE-REPLAY-HTTP-OBSERVATION-001

## Résultat

La tranche précédente s'appuyait sur le champ `upstream_read_attempts` du CLI. Ce runner ajoute une observation indépendante au point de sortie HTTP : Forgejo écoute sur `127.0.0.1:3131`, tandis qu'un proxy local borné occupe le port fixe de test `127.0.0.1:3130` et transmet les requêtes vers Forgejo. La réconciliation de deux replays manuels d'une issue privée a produit exactement un GET `200 /api/v1/repos/forge-admin/agent-forge-core/issues/1`. Le proxy a comparé l'en-tête d'autorisation en mémoire au token du lecteur non-admin; seul le booléen `reader_credential_match=true` est conservé. Le compteur déclaré par le CLI et le compte HTTP observé valent tous deux 1. Les deux lignes restent liées aux UUID demandés et au readback courant de l'issue; l'inbox ne change pas.

Le test du contrat de l'observateur refuse tout nombre de requêtes différent de 1, une méthode/route/statut inattendus, un autre credential ou des champs susceptibles de contenir des données sensibles. La suite ciblée passe 16/16. Le log TDD documente trois échecs avant l'ajout du validateur; après correction, les tests ciblés passent. La suite Python complète passe 180/180.

## Commande et versions

La commande est dans [command.txt](command.txt). L'instance a utilisé le binaire local Forgejo `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`, sur source auditée `0d791421f85ca13a63b665053d81ea765fbbc571`; la provenance du build ne rattache pas ce binaire à ce commit. Versions locales : Python 3.12.3, Cargo 1.95.0, Rust 1.95.0, Bubblewrap 0.9.0. Le binaire Forgejo rapporte avoir été construit avec Go 1.26.9; le binaire `go` autonome n'est pas installé dans l'environnement.

Le runner a reconstruit les deux binaires Rust hors ligne, `--locked`, sous Bubblewrap. `cargo fmt --all -- --check`, `git diff --check`, `py_compile`, le garde Cargo et ses 7 tests passent. `cargo metadata --locked --offline` confirme `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`. Aucune licence n'est attribuée. Les logs, receipts, JSON de readback, hashes et nettoyage sont dans ce répertoire; son manifeste est [SHA256SUMS](SHA256SUMS).

## Nettoyage et limites

Le hook a été désactivé avant l'arrêt de Forgejo. Forgejo sort avec le code 0; le proxy API, le proxy webhook et le receiver terminent; les readbacks ne trouvent aucun listener sur les ports 3130/3131 ou les ports temporaires suivis. Le dossier local d'état est conservé sous `/tmp/forgejo-native-replay-api-observation-001-state`.

Cette observation vérifie un seul dépôt, une issue et deux replays dans une instance jetable sous le même UID. Le proxy instrumente les requêtes pendant la fenêtre du CLI, mais n'est pas une frontière de sécurité pour une instance de production. Le binaire Forgejo n'est pas lié à son commit de source audité. Aucun LLM, AgentOS, runner CI, conflit concurrent, fusion ou comparaison de productivité/coût/tokens n'est exercé. Aucun service externe, coût, publication ou système de production n'est utilisé.

Prochaine tranche locale utile : provoquer un refus Forgejo réel `404` sur un dépôt privé après retrait de la permission read-only, puis vérifier par le même observateur le nombre de requêtes, le verdict inconclusif non-réessayable, l'absence de nouvel accès hors profil et l'invariance du journal. Le projet reste incomplet.
