/**
 * ListSingleChoicePage.java
 *
 * @author Created by Omnicore CodeGuide
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

