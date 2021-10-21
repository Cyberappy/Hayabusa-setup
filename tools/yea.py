import copy
from collections import OrderedDict
from io import StringIO

import yaml
from sigma.backends.base import SingleTextQueryBackend
from sigma.parser.condition import SigmaAggregationParser
from sigma.parser.modifiers.base import SigmaTypeModifier
from sigma.parser.modifiers.type import SigmaRegularExpressionModifier

class YeaBackend(SingleTextQueryBackend):
    """Base class for backends that generate one text-based expression from a Sigma rule"""
    ## see tools.py
    ## use this value when sigmac parse argument of "-t"
    identifier = "yea"
    active = True

    # the following class variables define the generation and behavior of queries from a parse tree some are prefilled with default values that are quite usual
    andToken = " and "                  # Token used for linking expressions with logical AND
    orToken = " or "                    # Same for OR
    notToken = " not "                  # Same for NOT
    regexExpression = "%s"
    subExpression = "(%s)"              # Syntax for subexpressions, usually parenthesis around it. %s is inner expression
    valueExpression = "%s"              # Expression of values, %s represents value
    typedValueExpression = dict()       # Expression of typed values generated by type modifiers. modifier identifier -> expression dict, %s represents value

    sort_condition_lists = False
    mapListsSpecialHandling = True

    name_idx = 1
    selection_prefix = "SELECTION_{0}"
    selections = []
    name_2_selection = OrderedDict()

    def __init__(self, sigmaconfig, options):
        super().__init__(sigmaconfig)

    def cleanValue(self, val):
        return val

    def generateListNode(self, node):
        return self.generateORNode(node)

    def generateMapItemNode(self, node):
        # 以下のルールに対応。
        # logsource:
        #   product: windows
        #   service: system
        # detection:
        # EventID: 7045
        # TaskName:
        #   - 'SC Scheduled Scan'
        #   - 'UpdatMachine'
        #
        # 変換されて以下の形式でnodeが渡される
        # - LogName System
        # - EventID 7045
        # - TaskName ['SC Scheduled Scan', 'UpdatMachine']
        fieldname, value = node
        name = self.selection_prefix.format(self.name_idx)

        if self.mapListsSpecialHandling == False and type(value) in (str, int, list) or self.mapListsSpecialHandling == True and type(value) in (str, int):
            childValue = None
            if type(value) == str and "*" in value[1:-1]:
                childValue = self.generateValueNode(value)
            elif type(value) == str and "*" in value:
                if value.startswith("*") and value.endswith("*"):
                    fieldname = fieldname + "|contains"
                    childValue = value[1:-1]
                elif value.endswith("*"):
                    fieldname = fieldname + "|startswith"
                    childValue = value[0:-1]
                elif value.startswith("*"):
                    fieldname = fieldname + "|endswith"
                    childValue = value[1:]
                else:
                    raise Exception("Value error")
            elif type(value) in (str, int):
                childValue = value
            else:
                childValue = self.generateNode(value)

            if name in self.name_2_selection:
                self.name_2_selection[name].append((fieldname, childValue))
            else:
                self.name_2_selection[name] = [(fieldname, childValue)]
            return name
        elif type(value) == list:
            return self.generateMapItemListNode(fieldname, value)
        elif isinstance(value, SigmaTypeModifier):
            return self.generateMapItemTypedNode(fieldname, value)
        else:
            self.name_2_selection[name].append((fieldname, None))
            return name


    def generateMapItemTypedNode(self, fieldname, value):
        # `|re`オプションに対応
        if type(value) == SigmaRegularExpressionModifier:
            name = self.selection_prefix.format(self.name_idx)
            field = fieldname + "|re"
            # TODO: ''の数が変になる
            val = self.regexExpression % value
            if name in self.name_2_selection:
                self.name_2_selection[name].append((field, val))
            else:
                self.name_2_selection[name] = [(field, val)]
        else:
            raise NotImplementedError("Type modifier '{}' is not supported by backend".format(value.identifier))
        return name

    def generateMapItemListNode(self, fieldname, value):
        ### 下記のようなケースに対応
        ### selection:
        ###     EventID:
        ###         - 1
        ###         - 2

        ### 基本的にリストはORと良く、generateListNodeもORNodeを生成している。
        ### しかし、上記のケースでgenerateListNode()を実行すると、下記のようなYAMLになってしまう。
        ### そうならないように修正している。
        ### なお、generateMapItemListNode()を有効にするために、self.mapListsSpecialHandling = Trueとしている
        ### selection:
        ###     EventID: 1 or 2
        name = self.selection_prefix.format(self.name_idx)
        self.name_idx += 1
        values = [ self.generateNode(value_element) for value_element in value]
        # selection下に置かれるもの
        if name in self.name_2_selection:
            self.name_2_selection[name].append((fieldname, values))
        else:
            self.name_2_selection[name] = [(fieldname, values)]
        return name

    def generateAggregation(self, agg):
        # python3 tools/sigmac -rI rules/windows/builtin/ --config tools/config/generic/powershell.yml --target yea > result.yaml
        if agg == None:
            return ""
        if agg.aggfunc == SigmaAggregationParser.AGGFUNC_COUNT:
            # Example rule: ./rules/windows/builtin/win_global_catalog_enumeration.yml
            raise NotImplementedError("COUNT aggregation operator is not yet implemented for this backend")
        if agg.aggfunc == SigmaAggregationParser.AGGFUNC_MIN:
            raise NotImplementedError("MIN aggregation operator is not yet implemented for this backend")
        if agg.aggfunc == SigmaAggregationParser.AGGFUNC_MAX:
            raise NotImplementedError("MAX aggregation operator is not yet implemented for this backend")
        if agg.aggfunc == SigmaAggregationParser.AGGFUNC_AVG:
            raise NotImplementedError("AVG aggregation operator is not yet implemented for this backend")
        if agg.aggfunc == SigmaAggregationParser.AGGFUNC_SUM:
            raise NotImplementedError("SUM aggregation operator is not yet implemented for this backend")
        if agg.aggfunc == SigmaAggregationParser.AGGFUNC_NEAR:
            # Example rule: ./rules/windows/builtin/win_susp_samr_pwset.yml
            raise NotImplementedError("NEAR aggregation operator is not yet implemented for this backend")


    def generateQuery(self, parsed):
        result = self.generateNode(parsed.parsedSearch)
        if parsed.parsedAgg:
            res = self.generateAggregation(parsed.parsedAgg)
            result += res
        self.selections.append(result)
        ret = ""
        with StringIO() as bs:
            ## 元のyamlをいじるとこの後の処理に影響を与える可能性があるので、deepCopyする
            parsed_yaml = copy.deepcopy(parsed.sigmaParser.parsedyaml)
            ## なんかタイトルは先頭に来てほしいので、そのための処理
            ## parsed.sigmaParser.parsedyamlがOrderedDictならこんなことしなくていい、後で別のやり方があるか調べる
            bs.write("title: " + parsed_yaml["title"]+"\n")
            del parsed_yaml["title"]

            ## detectionの部分だけ変更して出力する。
            parsed_yaml["detection"] = {}
            parsed_yaml["detection"]["condition"] = self.andToken.join(self.selections)
            for key, values in self.name_2_selection.items():
                parsed_yaml["detection"][key] = {}
                for fieldname, value in values:
                    parsed_yaml["detection"][key][fieldname] = value

            yaml.dump(parsed_yaml, bs, indent=4, default_flow_style=False)
            ret = bs.getvalue()

        return ret
