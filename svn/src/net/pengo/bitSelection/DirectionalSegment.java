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

/*
 * DirectionalSegment.java
 *
 * Created on 28 November 2004, 23:16
 */

package net.pengo.bitSelection;

/**
 *
 * @author Peter Halasz
 */
public class DirectionalSegment extends BitSegment {
    private boolean isFacingRight;
    
    /** Creates a new instance of DirectionalSegment */
    public DirectionalSegment(long head, long tail) {
        this(new BitCursor(head,0), new BitCursor(tail,0));
    }
    
    public DirectionalSegment(BitCursor head, BitCursor tail) {
        super(head,tail);
        if (head == lastIndex) {
            isFacingRight = true;
        } else {
            isFacingRight = false;
        }
    }
    
    // or lead
    public BitCursor getHead() {
        if (isFacingRight) {
            return lastIndex;
        } else {
            return firstIndex;
        }
    }

    // or anchor
    public BitCursor getTail() {
        if (!isFacingRight) {
            return lastIndex;
        } else {
            return firstIndex;
        }
    }

    public String toString() {
        return getHead() + "-" + getTail();
    }

    
    //fixme: for completeness, override subtract method (from BitSegment) to preserve direction
}
