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
 * A ColoumnGroup is an drawing area where _one_ style of data (eg hex) is drawn. (but many rows/columns)
 *
 * a default ColumnGroup will be made for repeating a single unit.
 *
 * @author Peter Halasz
 */

package net.pengo.hexdraw.tabled;

import java.util.*;
//import javax.swing.table.*;

//FIXME: make this to extend Panel or something drawable
public abstract class ColumnGroup
{
	
	private float ptSize = 9; // this is the scale //FIXME:
	private boolean isPtSizeFixed = false; //setWidth (of setSize) should change the number of units OR the ptSize depending on isPtSizeFixed
	
	
	// number of hex units to draw per line. aka column count
	private int unitsPerLine;
	
	//FIXME: make default setSize method.
	//FIXME: make convinence setSize(height, units) method
	
	public void setUnitsPerLine(int unitsPerLine)
	{
		this.unitsPerLine = unitsPerLine;
	}
	

	
	public int getUnitsPerLine()
	{
		return unitsPerLine;
	}
	
	// min
	// for calculating a new size for everyone
	abstract public float widthIf(int unitsPerLine);
	
	abstract public float widthIf(int unitsPerLine, int ptSize);
	
    // is it this column group that's been selected?
    abstract public boolean getIsPrimarySelection();
    abstract public void setIsPrimarySelection(boolean primary);
	

    
	//TableColumn getColumnClass(int col);
	
    
}

