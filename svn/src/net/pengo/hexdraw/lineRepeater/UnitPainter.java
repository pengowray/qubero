/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/**
 * UnitPainter.java
 *
 * @author Peter Halasz
 */

package net.pengo.hexdraw.lineRepeater;

/** paint a single "unit". all components within the unit draw the same value. */
import java.awt.*;

class UnitPainter extends Container {
    public void addImpl(Component comp, Object constraints, int index) {
        // check that component is a unit-ish component?
        // or wrap it in a UnitPainter
        super.addImpl(comp,constraints,index);
    }
    
    public void paintComponents(Graphics g, UnitInfo unit) {
        Component[] com = getComponents();
        for (int i=0; i<com.length; i++) {
            if (com[i] instanceof HexUnit) { // xxx HexUnit or another class ???
                ((HexUnit)com[i]).paint(g, unit);
            } else {
                com[i].paint(g);
            }
        }
    }
}


