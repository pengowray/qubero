/**
 * QNodePage.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.propertyEditor;
import net.pengo.dependency.QNode;
import javax.swing.JTextPane;
import net.pengo.resource.QNodeResource;



public class QNodePage extends PropertyPage {
    private QNodeResource res;
    
    /** Creates a new instance of SummaryPage */
    public QNodePage(QNodeResource res) {
        super();
        this.res = res;
        build();
    }
    
    public void build() {
        removeAll();
        //add(new JLabel( "Value: " + res.getValue().toString() ));
        //add(new JLabel( "Size (bytes): " + res.getSelectionData().getLength() ));
        
        //add(new JLabel( "Value: " + res.getValue().toString() ));
        JTextPane jtp = new JTextPane();
        String s = "toString(): " + res + "\n\n" +
			"Sinks (dependents): " + res.getSinkCount() + "\n" +
			"Sources: " + res.getSources().length + "\n"; //xxx: list them!
			
        jtp.setText(s) ;
        add(jtp);
    }
    
    public void save() {
        return;
    }
    
    public String toString() {
        return "Dependency Info";
    }
}
