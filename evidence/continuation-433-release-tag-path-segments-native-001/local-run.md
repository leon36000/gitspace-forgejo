# CONTINUATION-433 — URL de tag Release à segments dans le parcours natif

Date : 2026-10-28. Worktree local/WEBHOOK-005-canonical-links, HEAD f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0. Aucun transfert, changement de ref distante, publication, coût ou accès production dans ce lot.

## Écart et correction

CONTINUATION-432 a corrigé le reconciler Rust, mais la vérification de composition Python dans scripts/run_forgejo_live_webhook_reconcile_emitted.py utilisait encore quote(tag_name, safe=""), encodant le tag complet comme un seul segment. Le test nouveau a échoué avant correction avec « current release URL is not derived from the configured repository and tag » pour feature/v1.0; le journal est red-python-test.log et le code de retour 1 est dans red.exit.

La validation découpe maintenant le tag sur /, applique Python quote à chaque segment avec le jeu sûr Go de path-segment $&+:@= et réassemble les segments avec /. L’entrée de test du flux de publication utilise le tag jetable codex-e2e/release-<nonce>, afin que le parcours natif couvre cette forme lors de ses prochaines exécutions. Les tests de composition couvrent aussi + conservé, ; encodé et un caractère UTF-8 encodé en octets. Cela correspond à Forgejo v17, commit source 0d791421f85ca13a63b665053d81ea765fbbc571 : modules/util/url.go, PathEscapeSegments, découpe/réassemble le chemin; models/repo/release.go, Release.HTMLURL, l’emploie pour /releases/tag/; services/convert/release.go expose HTMLURL dans l’API. Forgejo go.mod déclare go 1.26.0 et toolchain go1.26.9. Le code Go officiel de net/url montre que PathEscape sur un segment échappe / ; , ? mais conserve : @ & = + $, ce que reflète la validation Python : https://go.dev/src/net/url/gen_encoding_table.go#L290-L304.

## Preuve de composition et parcours natif

Le test rouge/vert de la classe ReleaseReconciliationCompositionTests passe après correction : 9/9. Il vérifie les quatre tags feature/v1.0, feature/a+b, feature/a;b et feature/café. L’analyse AST des deux fichiers Python passe. Les fichiers scripts/tests/matrice non suivis par Git ont aussi été contrôlés explicitement pour les espaces finaux et le saut de ligne final par check_touched_whitespace.py; git diff --check seul ne couvre pas ces fichiers non suivis.

L’image Docker locale préexistante codeberg.org/forgejo/forgejo:17.0.0 a l’identifiant sha256:06d38310de496a10186d952bdad5ca764c153c8023ede8dfa0944c79a1dc5412. Le binaire extrait sous --network none a le SHA-256 e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7 et rapporte Forgejo 17.0.0+gitea-1.22.0, compilé avec go1.26.9. Le commit Go exact correspondant au binaire de l’image n’est pas établi; il est séparé de l’audit source Forgejo au commit cité plus haut.

Le runner natif principal, avec --release-webhook, publie le Release et reçoit les webhooks de l’instance jetable. Le résultat est verdict=passed. Huit deliveries natives (issues, release, pull_request et commentaire) sont toutes livrées et acceptées en HTTP 202. Le Release tag codex-e2e/release-a1ed07e486 est l’événement published, séquence 5, ID 1. Le vrai CLI Rust lit l’API courante une fois, retourne current_state_read/present et produit l’URL /releases/tag/codex-e2e/release-a1ed07e486 avec le slash conservé entre segments.

Après redémarrage du receiver avec le même journal, le rejeu Release retourne HTTP 202 avec duplicate=true; les 8 enregistrements, les 61 844 octets et le SHA-256 du journal restent identiques. Le credential de lecture invalide produit inconclusive/upstream_unauthorized. Le hook est désactivé, les processus suivis sont terminés et les ports suivis sont absents. La PR du parcours est fermée sans fusion. Le build Rust du runner était Bubblewrap avec réseau/PID non partagés, environnement vidé, sources/registre/sysroot en lecture seule, Cargo offline/locked; le staging hôte confirme tmpfs puis suppression après compilation.

Les reçus natifs, la provenance, les sorties CLI et le manifeste de run sont sous native-run/. Le manifeste local de ce sous-dossier vérifie 19/19 fichiers; le manifeste global append-only est mis à jour pour les fichiers de code, checkpoint, matrice et preuves de ce lot. Le statut de licence Cargo n’a pas changé : license=null, license_file=null, publish=[], no_license_selected, droits not_assessed, NOASSERTION; la garde passe 12/12.

## Portée et limites

Ce run démontre spécifiquement la chaîne Release natif → receiver → réconciliation Rust → validation Python pour un tag à slash sur une image Forgejo locale. Il ne transforme pas le parcours complet produit en workflow agent : aucun LLM, AgentOS, RAG ou mémoire n’a été utilisé; aucune CI conventionnelle n’a été comparée, aucune preuve indépendante n’a autorisé une fusion, et la PR est fermée sans merge. n=1 et l’image runtime n’établit pas un commit exact de source Forgejo. Aucun code Forgejo/GitSpace n’est copié ni licencié par ce lot.

La matrice de refus a été corrigée pour retirer l’ancien constat périmé selon lequel le runner principal n’émettait pas de Release. Le projet reste incomplet.
