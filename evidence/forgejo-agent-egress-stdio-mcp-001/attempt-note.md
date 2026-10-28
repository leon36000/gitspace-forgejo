# Tentative préliminaire conservée — FORGEJO-AGENT-EGRESS-STDIO-MCP

Le 26 octobre 2026, une première invocation du runner a redirigé ses propres sorties `stdout` et `stderr` dans son répertoire d’evidence avant son démarrage. Le préflight refuse intentionnellement un répertoire contenant des artefacts inattendus; le processus a donc terminé avec `RuntimeError` avant de créer `/tmp/forgejo-agent-egress-stdio-mcp-001` ou de démarrer Forgejo. La sortie JSON et le fichier stderr vide sont conservés ici.

Ce n’est pas un échec du test d’isolation ni une preuve produit. La commande a été relancée sans rediriger dans le répertoire d’evidence; le run reproductible réussi se trouve dans [FORGEJO-AGENT-EGRESS-STDIO-MCP-005](../forgejo-agent-egress-stdio-mcp-005/local-run.md).
