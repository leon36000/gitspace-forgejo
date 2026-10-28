# Diagnostic conservé — première suite du contrat capacités

Commande : cargo test --locked --offline --bin forgejo-reconcile-local.

Le premier run suivant l’ajout du refus a échoué dans tests::capabilities_list_exact_merge_target_profiles : l’objet produit par capabilities_document()["credential_boundary"] ne contenait pas pipe_read_failure, tandis que l’attente du contrat l’exigeait. Le run a ensuite été corrigé en ajoutant au document public le code de refus, l’issue inconclusive_without_upstream_read, le type de liaison single/batch et l’absence du credential dans la réponse. Le rerun du binaire passe 161/161 et le run --all-targets passe 688/0/145 ignorés. Le journal du premier run a été remplacé par le journal vert final; ce résumé préserve le diagnostic observé.
