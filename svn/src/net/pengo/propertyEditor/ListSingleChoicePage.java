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
 * ListSingleChoicePage.java
 *
 * @author Peter Halasz
 */

package net.pengo.propertyEditor;
import net.pengo.resource.ListSingleChoiceResource;
import javax.swing.JLabel;
import javax.swing.JComboBox;
import net.pengo.resource.IntPrimativeResource;



public class ListSingleChoicePage extends EditablePage {
    
    
    private ListSingleChoiceResource choiceRes;
    private JComboBox choiceBox;
    
    public ListSingleChoicePage(ListSingleChoiceResource choiceRes, PropertiesForm form) {
	super(form);
	this.choiceRes = choiceRes;
	
	add(new JLabel( "Selection: " ));
	choiceBox = new JComboBox(choiceRes.getList().toArray());
	choiceBox.addActionListener( getModActionListener() );
	add(choiceBox);
	build();
    }
    
    public ListSingleChoicePage(ListSingleChoiceResource choiceRes) {
	this(choiceRes, null);
    }
    
    
    protected void saveOp() {
	//fixme: i dunno? can't we do this without creating a new resource?
	choiceRes.setChoice( new IntPrimativeResource(choiceBox.getSelectedIndex()) );
    }
    
    public void buildOp() {
	choiceBox.setSelectedIndex( choiceRes.getChoice().toInt() );
    }
    
    /**
     * Method toString
     *
     * @return   a String
     *
     */
    public String toString() {
	// TODO
	return "Hello!";
    }
    
}

