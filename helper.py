import json
from copy import deepcopy

def merge_models(a, b):
    """
    Deep merge dict a into dict b (mutates b).
    - For dicts: recurse.
    - For lists of primitives: take union (set).
    - For lists of objects: merge corresponding items by index (if both exist),
      otherwise append.
    - For primitives: b[k] = v (overwrite).
    """
    if not isinstance(a, dict) or not isinstance(b, dict):
        return b

    for k, v in a.items():
        if k not in b:
            b[k] = deepcopy(v)   # avoid sharing references
        elif isinstance(v, dict) and isinstance(b[k], dict):
            merge_models(v, b[k])
        elif isinstance(v, list) and isinstance(b[k], list):
            # Check if both lists contain only primitives (not dict/list)
            def is_primitive_list(lst):
                return all(not isinstance(item, (dict, list)) for item in lst)

            if is_primitive_list(v) and is_primitive_list(b[k]):
                # Union of unique values (order not preserved)
                b[k] = list(set(b[k] + v))
            else:
                # For lists of objects: merge by index
                # If b is shorter, extend; if v has more, append.
                for i, item in enumerate(v):
                    if i < len(b[k]):
                        if isinstance(item, dict) and isinstance(b[k][i], dict):
                            merge_models(item, b[k][i])
                        else:
                            # If item is primitive or b[k][i] is different type,
                            # we overwrite or append? Here we simply replace.
                            b[k][i] = deepcopy(item)
                    else:
                        b[k].append(deepcopy(item))
        else:
            # Overwrite if types differ or primitive
            b[k] = deepcopy(v)
    return b

def extract_models_schema(data):
    """
    Traverse JSON and return a list of all objects under any key named "models".
    """
    models = []
    if isinstance(data, dict):
        for k, v in data.items():
            if k == "models" and isinstance(v, dict):
                # v is { "model_id": { ... } }
                for _, model_def in v.items():
                    models.append(model_def)
            else:
                models.extend(extract_models_schema(v))
    elif isinstance(data, list):
        for item in data:
            models.extend(extract_models_schema(item))
    return models

if __name__ == "__main__":
    with open('api.json') as f:
        data = json.load(f)

    models = extract_models_schema(data)
    if not models:
        print("No models found in api.json")
        exit(1)

    # Start with the first model as the base
    base_model = deepcopy(models[0])

    # Merge all other models into base
    for model in models[1:]:
        merge_models(model, base_model)

    with open('base_model.json', 'w') as f:
        json.dump(base_model, f, indent=2)
    print("Merged schema written to base_model.json")
