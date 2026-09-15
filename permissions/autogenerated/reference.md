## Default Permission

Allows reading the system power state (power source and battery
information) through the get_power_state command.

This plugin only observes the system; it never controls power, so the
default permission set exposes the read-only query and nothing else.

#### This default permission set includes the following:

- `allow-get-power-state`

## Permission Table

<table>
<tr>
<th>Identifier</th>
<th>Description</th>
</tr>


<tr>
<td>

`power-monitor:allow-get-power-state`

</td>
<td>

Enables the get_power_state command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`power-monitor:deny-get-power-state`

</td>
<td>

Denies the get_power_state command without any pre-configured scope.

</td>
</tr>
</table>
