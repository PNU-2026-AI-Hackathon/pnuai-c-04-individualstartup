# Mechanical Part Family Generation Queries

## 1. Gear

기능적으로 사용할 수 있는 **spur gear**를 만들어줘.
이빨 수는 **24개**, module은 **2 mm**, pressure angle은 **20°**, face width는 **10 mm**,*중심 bore diameter는 **8 mm**로*해줘.

주어진 설계값에 맞는 involute tooth profile을 사용하고, pitch diameter와 전체 외경 등 나머지 치수는 기어 형상 관계에 맞게 결정해줘. 각 tooth가 균일하게 배치되고 실제 다른 동일 module의 gear와 맞물릴 수 있는 형상으로 만들어줘.

## 2. Shaft

기능적으로 사용할 수 있는 **stepped shaft**를 만들어줘.
전체 길이는 **100 mm**이며,*세*구간의 직경은 각각 **20 mm, 30 mm, 20 mm**로*만들어줘.*중앙의 직경 30 mm 구간 길이는 **40 mm**로 하고, 한쪽 끝에는 **폭 6 mm의 keyway**를 만들어줘.

각 직경 변화 구간에는 적절한 shoulder와 fillet을 적용하고, 회전축 부품으로 사용하기 자연스러운 형상으로 세부 치수를 결정해줘.

## 3. Bracket

기계 부품을 고정할 수 있는 **L-shaped mounting bracket**을 만들어줘.
Base plate 크기는 **80 × 50 mm**, vertical plate*높이는 **60 mm**, plate thickness는 **6 mm**로*해줘.

Base에는 **직경 8 mm의 mounting hole 4개**, vertical plate에는 **직경 12 mm의*중심 hole 1개**를*만들어줘. 필요한 fillet이나 보강 rib의 크기는 구조적으로 자연스럽게 결정해줘.

## 4. Housing

회전축 또는 bearing을 지지할 수 있는 **mechanical housing**을 만들어줘.
전체 크기는 약 **100 × 80 × 60 mm**로 하고, 중앙에는 **직경 40 mm의 cylindrical bore**를 만들어줘.

하부에는 장착을 위한 flat base와 **직경 8 mm의 mounting hole 4개**를 배치하고, bore 주변에는 충분한 wall thickness를 유지해줘. 외부 형상과 fillet, boss 등 세부 요소는 실제 기계 housing처럼 자연스럽게 설계해줘.

## 5. Flange / Hub

축과 다른 부품을 결합하기 위한 **circular flange hub**를 만들어줘.
Flange outer diameter는 **80 mm**, thickness는 **10 mm**,*중앙 bore diameter는 **20 mm**로*해줘.

Pitch circle diameter **60 mm** 위치에 **직경 8 mm의 bolt hole 6개**를 원주 방향으로 균등하게 배치해줘. 중앙에는 shaft 결합을 위한 cylindrical hub를 추가하고, hub의 외경과 길이는 전체 형상에 맞게 결정해줘.

## 6. Fastener

기계 조립에 사용할 수 있는 **hex-head bolt**를 만들어줘.
Nominal thread diameter는 **M10**,*전체*길이는 **50 mm**, thread pitch는 **1.5 mm**로*해줘.

일반적인 육각 bolt head를 가지고 있어야 하며, shaft 일부에는 실제 나사 체결이 가능한 metric thread profile을 만들어줘. Head 크기, chamfer, thread 길이 등 나머지 치수는 일반적인 M10 bolt 비율에 맞게 결정해줘.

## 7. Bushing / Sleeve

축을 지지하거나 마찰을 줄이기 위한 **flanged bushing**을 만들어줘.
Inner bore diameter는 **20 mm**, body outer diameter는 **30 mm**, body length는 **30 mm**로*해줘.

한쪽 끝에는 outer diameter **40 mm**, thickness **5 mm**의 flange를*추가해줘.*중심축을 기준으로 동심 형상을 유지하고, edge에는 적절한 chamfer나 fillet을 적용해줘.

## 8. Spring

압축 하중을 받을 수 있는 **helical compression spring**을 만들어줘.
Wire diameter는 **3 mm**, mean coil diameter는 **24 mm**, active coil*수는 **8개**, free length는 **50 mm**로*해줘.

일정한 pitch를 가진 원통형 helix 형태로 만들고, 양 끝단은 압축 spring으로 사용할 수 있도록 자연스럽게 마무리해줘. Coil 간 간격은 free length와 coil 수에 맞게 결정해줘.

## 9. Pulley

V-belt 동력 전달에 사용할 수 있는 **single-groove V-belt pulley**를 만들어줘.
Pulley outer diameter는 **80 mm**, width는 **20 mm**,*중심 bore diameter는 **15 mm**로*해줘.

외주에는 하나의 V-shaped belt groove를 만들고, 중심에는 shaft 결합을 위한 hub를 포함해줘. Groove angle과 depth, hub 크기 및 나머지 세부 치수는 실제 belt가 안정적으로 걸릴 수 있도록 합리적으로 결정해줘.

## 10. Cam

회전 운동을 follower의 왕복 운동으로 변환할 수 있는 **radial disk cam**을 만들어줘.
Base circle radius는 **25 mm**,*최대 lift는 **15 mm**, cam thickness는 **10 mm**,*중심 bore diameter는 **10 mm**로*해줘.

한 회전 동안 follower가 0에서 최대 lift까지 상승한 뒤 다시 원래 위치로 돌아오는 연속적인 cam profile을 만들어줘. Cam profile은 급격한 불연속이 없도록 부드럽게 연결하고, 실제 회전 부품으로 사용할 수 있는 형태로 만들어줘.
