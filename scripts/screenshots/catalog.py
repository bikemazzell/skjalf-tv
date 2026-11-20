"""Made-up catalog for the README screenshots (shared by assets and server)."""

LIVE_GROUPS = [("1", "News"), ("2", "Sports"), ("3", "Movies & Drama"), ("4", "Kids"), ("5", "Documentary"), ("6", "Music")]
CHANNELS = [  # id, number, name, group
    ("101", 1, "Northwind News 24", "1"), ("102", 2, "Harbor City Today", "1"),
    ("103", 3, "Global Ledger", "1"), ("104", 4, "Morning Signal", "1"),
    ("105", 5, "Cedar Valley Local", "1"),
    ("201", 10, "Summit Sports", "2"), ("202", 11, "Arena One", "2"),
    ("301", 20, "Cinema Lane", "3"), ("401", 30, "Little Comets", "4"),
    ("501", 40, "Deep Earth", "5"), ("601", 50, "Night Owl FM", "6"),
]
VOD_GROUPS = [("11", "Drama"), ("12", "Thriller"), ("13", "Science Fiction"), ("14", "Comedy")]
MOVIES = [  # id, name, group, rating, added, year, genre, minutes
    ("1001", "The Lantern Keeper (2025)", "11", "7.8", 1763500000, "2025", "Drama, Mystery", 112),
    ("1002", "Salt and Iron (2024)", "12", "7.1", 1763400000, "2024", "Thriller", 104),
    ("1003", "Paper Moons (2026)", "14", "6.9", 1763300000, "2026", "Comedy, Romance", 97),
    ("1004", "Beyond the Ninth Wave (2026)", "13", "8.2", 1763200000, "2026", "Science Fiction, Adventure", 128),
    ("1005", "Rivers of Ash (2023)", "11", "7.4", 1763100000, "2023", "Drama, War", 121),
    ("1006", "Glass Harbor (2025)", "12", "7.0", 1763000000, "2025", "Thriller, Crime", 109),
    ("1007", "The Quiet Orchard (2024)", "11", "7.6", 1762900000, "2024", "Drama, Family", 101),
]
SERIES_GROUPS = [("21", "Drama"), ("22", "Crime"), ("23", "Science Fiction")]
SERIES = [  # id, name, group, rating, modified
    ("2001", "Ravenhold (2025)", "21", "8.4", 1763500000),
    ("2002", "The Ferry Murders (2025)", "22", "7.6", 1763400000),
    ("2003", "Starfall Station (2026)", "23", "8.1", 1763300000),
    ("2004", "Kingdom of Ice (2024)", "21", "8.7", 1763200000),
    ("2005", "Harbor Lights (2025)", "21", "7.3", 1763100000),
]
# Two-colour palettes for posters and logos, chosen per item.
PALETTES = [((32, 58, 96), (196, 120, 52)), ((24, 70, 62), (220, 196, 120)), ((80, 28, 52), (232, 150, 120)),
            ((20, 30, 70), (120, 170, 230)), ((60, 40, 20), (230, 180, 90)), ((30, 60, 40), (170, 220, 140)),
            ((70, 20, 20), (240, 120, 80))]
