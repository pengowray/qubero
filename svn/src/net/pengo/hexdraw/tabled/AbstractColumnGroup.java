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
 * AbstractColumnGroup.java
 *
 * A bunch of columns (or one column). Usually with the same renderer.
 *
 * @author Peter Halasz
 */

package net.pengo.hexdraw.tabled;

import javax.swing.table.*;
import java.awt.*;

public abstract class AbstractColumnGroup
{
    abstract public int getColumnCount();
    //abstract public Column getColumn(int col);
    
    abstract public int getPreferredWidth();
    abstract public int getMaximumWidth();
    abstract public int getMinimumWidth();
    
    // set width of entire column group
    //public void setWidth(...);
    
    // set width of specific column.. may or may not affect column group width
    //public void setColumnWidth();
	
}

