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
 * ColumnGroupManager.java
 *
 * @author  Peter Halasz
 */
	
package net.pengo.hexdraw.tabled;



import java.util.*;
import javax.swing.table.*;

//JTableHeader
//TableColumnModel

public class GroupColumnModel extends DefaultTableColumnModel
{
    public static final int ADDR = 1;
    public static final int HEX = 2;
    public static final int ASCII = 3;
    public static final int GREY = 4;

    private ArrayList groupList = new ArrayList();
    private int unitsPerLine;
	
    public GroupColumnModel(int unitsPerLine) {
	super();
	
	this.unitsPerLine = unitsPerLine;
    }
	
    // returns itself (for chaining).
    
    // commented out to make compile
    /*
    public GroupColumnModel addColumnGroup(int type) {
	ColumnGroup cg = null;
	
	switch(type) {
	    case ADDR:
		cg = new AddressColumnGroup(unitsPerLine);
		break;
	    case HEX:
		cg = new HexColumnGroup(unitsPerLine);
		break;
	    case ASCII:
		cg = new AsciiColumnGroup(unitsPerLine);
		break;
	    case GREY:
		cg = new GreyColumnGroup(unitsPerLine);
		break;
	}
	
	groupList.add(cg);
	List c = cg.getColumns();
	tableColumns.addAll(c);
	return this;
    }
	*/
    
    
}

