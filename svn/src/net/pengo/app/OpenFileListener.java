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
 * OpenFileListener.java
 *
 * Created on 15 August 2002, 20:14
 */


/**
 *
 * @author  Peter Halasz
 */
package net.pengo.app;
import net.pengo.data.*;

public interface OpenFileListener extends java.util.EventListener {

    
    public void fileSaved(FileEvent e);
    public void fileClosed(FileEvent e);
    
    public void selectionMade(SelectionEvent e);
    public void selectionCleared(SelectionEvent e);

    public void selectionCopied(ClipboardEvent e);
        
    public void dataEdited(DataEvent e);
    public void dataLengthChanged(DataEvent e);
    
    //FIXME: remove  made/removed events (use resource listener instead)
    
    //FIXME: vetoable events?
    //FIXME: split this up some?
    
}
