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
 * DefaultDefinitionResource.java
 
 DefaultDefinitionResource is an "untyped" resource. (or not yet typed).
 It is based on a selection.
 
 XXX: Perhaps this should be mereged with DefaultSelectionResource as
 this class is converted to that one when it is used.
 
 *
 * Created on 21 August 2002, 18:18
 */

package net.pengo.resource;

import javax.swing.JMenu;
import javax.swing.JSeparator;

import net.pengo.pointer.JavaPointer;

/**
 * This one's going the way of the dodo
 *
 * @author  Peter Halasz
 */
public class DefaultDefinitionResource extends DefinitionResource implements AddressedResource
{
    private int sinkCount;
    
    public final JavaPointer selResP = new JavaPointer("net.pengo.resource.SelectionResource"); //private SelectionResource selRes;
    
    //final protected LongListSelectionModel sel;
    
    public Resource[] getSources() {
        return new Resource[]{};
    }
    
    public void editProperties(){};
    
    public DefaultDefinitionResource(SelectionResource sel) {
        super();
        selResP.setValue(sel);
    }

    public void giveActions(JMenu menu) {
        super.giveActions(menu);
        menu.add(new JSeparator());

        TypeRegistry.instance().giveConversionActions(menu, getSelectionResource());
        //menu.add(new JSeparator());
    }
    
    public String valueDesc() {
        return selResP.evaluate().valueDesc();
    }    
    
    public void doubleClickAction() {
        // duplicated in BooleanAddrssdResource
        super.doubleClickAction();
        getSelectionResource().makeActive();
    }
    
    
    
    public net.pengo.restree.ResourceList getSubResources() {
        return null;
    }
    
    
    
    public void removeSink(Resource sink) {
        sinkCount--;
    }
    
    public int getSinkCount() {
        return sinkCount;
    }
    
    public void addSink(Resource sink) {
        sinkCount++;
    }

    public SelectionResource getSelectionResource() {
        return (SelectionResource)selResP.evaluate();
    }
    
    public void setSelectionResource(SelectionResource selRes) {
        selResP.setValue(selRes);
    }
    
    
}
