# 检测pumpfun新铸造代币\监控\自动买入\自动卖出，使用rust编写

# 借鉴了[Allen-Taylor/pump_fun_py](https://github.com/Allen-Taylor/pump_fun_py)的部分代码
# 以下是初始参数设定，仅供参考，需要你自己研究更好的规则和参数（初始买入和卖出逻辑经过我的测试会出现持续亏损，慎用!）
## 1.检测Pump.fun Token Mint Authority最铸币信息，获取铸币地址
## 2.进行为期10分钟的价格稳定检测，如果检测期内价格下跌超过40%就淘汰，如果超过检测期执行下一步
## 1.如果价格未满足条件（大于0.000012 且 小于0.000015）就停止检测，否则进入下一道检测程序
## 2.持有者分布检测，如果 持有2%的地址大于等于5 或 持有3%的地址大于等于3 或 持有4%的地址大于等于3 或 持有5%的地址大于等于2 或 持有6%的地址大于等于1 就返回false，买入中断；否则返回true，进入下一道检测程序
## 3.关键词检测，如果名中关键词买入中断，否则执行下一步
## 5.执行买入操作并发送到discord，进入持有期并持续检测
## 6.卖出逻辑：持有期内如果30分钟内达到设定目标价格就卖出100%并发送到discord，超过30分钟未达到设定目标价格卖出100%
