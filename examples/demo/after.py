def score(values):
    return sum(values) / len(values)


def normalize(records):
    return [record.strip() for record in records]


def report(values):
    clean = normalize(values)
    return {
        "mean": score(clean),
        "count": len(clean),
    }
