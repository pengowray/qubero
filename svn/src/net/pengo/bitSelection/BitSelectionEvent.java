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

// similiar to javax.swing.event.ListSelectionEvent

package net.pengo.bitSelection;

import java.util.EventObject;


//FIXME: add type of change in here too?

public class BitSelectionEvent extends EventObject {
    private boolean isAdjusting;
    private BitSegment changeRange;

    
    public BitSelectionEvent(Object source, BitSegment changeRange, boolean isAdjusting) {
        super(source);
        this.changeRange = changeRange;
        this.isAdjusting = isAdjusting;
    }
    
    public boolean getValueIsAdjusting() {
        return isAdjusting;
    }

    public BitSegment  getChangeRange() {
        return changeRange;
    }
    
    public String toString() {
        String properties = " source=" + getSource() + " changeRange= " + changeRange + " isAdjusting= " + isAdjusting;
        return getClass().getName() + "[" + properties + "]";
    }
    
    //fixme: what's this meant to do? should only be in the listener?
    public void valueChanged(BitSelectionEvent e) {
        if (e.getSource() == this) {
            return;
        }
    }
}


