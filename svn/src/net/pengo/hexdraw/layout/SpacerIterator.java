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
 * SpacerIterator.java
 *
 * Created on 10 December 2004, 07:02
 */

package net.pengo.hexdraw.layout;

import java.awt.Point;

import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;

/**
 *
 * @author Peter Halasz
 */
public interface SpacerIterator  { // extends Iterator<SuperSpacer>

    public void init(BitSegment range, long startPos);

    boolean hasNext(BitCursor bits);
    SuperSpacer next(BitCursor bits);
    void remove();
    
    SuperSpacer currentSpacer();
    long currentIndex();
    Point currentStartPoint();
    Point nextStartPoint(BitCursor bits);

    public int getXChange();
    public int getYChange();
    //public int getTotalXChange();
    //public int getTotalYChange();
    
    public void jumpToNext(long nextPos, BitCursor bits);
    
    public BitCursor getBitOffset();    
}
