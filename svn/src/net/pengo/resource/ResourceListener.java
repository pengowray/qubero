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
 * @author  Pengo
 */
package net.pengo.resource;

public interface ResourceListener extends java.util.EventListener {
    public void resourceAdded(ResourceEvent e);
    public void resourceRemoved(ResourceEvent e);
    public void resourceMoved(ResourceEvent e);
    public void resourceValueChanged(ResourceEvent e);
    public void resourcePropertyChanged(ResourceEvent e);
    public void resourceTypeChanged(ResourceEvent e);
    
    //FIXME: events to allow vetoing?
    
    //FIXME: bulk events

}
