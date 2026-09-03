def normalize(items):
    return [item.strip() for item in items]


def score(values):
    return sum(values) / len(values)


def report(values):
    clean = normalize(values)
    return {"mean": score(clean), "count": len(clean)}
